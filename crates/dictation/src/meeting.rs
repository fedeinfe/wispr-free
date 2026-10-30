//! Starting and stopping meeting notes, and the suggestion to do so (spec §4). The recording and
//! what happens to it afterwards belong to `wf_meetings`. This is only what the running app has
//! to do around it: a thread that keeps the meeting away from the dictation flow's own work.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use wf_audio::LevelMeter;
use wf_core::audio::FRAME_MS;
use wf_core::settings::Settings;
use wf_meetings::{
    Channel, MeetingConfig, MeetingDetector, MeetingEvent, MeetingRecorder, MicClient, mic_clients,
    other_app_playing, repair_meeting,
};

use crate::Event;
use crate::coordinator::Input;

/// How often a recording's events are read.
const EVENT_POLL: Duration = Duration::from_millis(100);
/// How often the OS is asked who holds the mic, when the user opted in.
const DETECT_POLL: Duration = Duration::from_secs(2);
/// Frames of pure digital silence from the system channel, while something plays, before the
/// silence counts as a denied permission: 5 s.
const SILENT_FRAMES: u32 = 5_000 / FRAME_MS;

const RECORDING_EXTENSION: &str = "recording";
const FINISHED_EXTENSION: &str = "ogg";

#[cfg(target_os = "macos")]
const SILENT_SYSTEM_AUDIO: &str =
    "System audio is silent. Allow System Audio Recording in Privacy & Security";
#[cfg(not(target_os = "macos"))]
const SILENT_SYSTEM_AUDIO: &str = "System audio is silent";

/// A meeting being written to disk.
pub trait MeetingTape: Send {
    fn events(&self) -> &Receiver<MeetingEvent>;
    fn stop(self: Box<Self>) -> Result<(), String>;
}

impl MeetingTape for MeetingRecorder {
    fn events(&self) -> &Receiver<MeetingEvent> {
        MeetingRecorder::events(self)
    }

    fn stop(self: Box<Self>) -> Result<(), String> {
        MeetingRecorder::stop(*self)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

/// The OS, as far as meetings go.
pub trait MeetingRoom: Send + Sync {
    fn record(&self, path: PathBuf, ranking: Vec<String>) -> Result<Box<dyn MeetingTape>, String>;
    fn mic_clients(&self) -> Vec<MicClient>;
    fn other_app_playing(&self) -> bool;
}

pub struct SystemMeetingRoom;

impl MeetingRoom for SystemMeetingRoom {
    fn record(&self, path: PathBuf, ranking: Vec<String>) -> Result<Box<dyn MeetingTape>, String> {
        let config = MeetingConfig {
            mic_ranking: ranking,
            ..MeetingConfig::new(path)
        };
        match MeetingRecorder::start(config) {
            Ok(recorder) => Ok(Box::new(recorder)),
            Err(error) => Err(error.to_string()),
        }
    }

    fn mic_clients(&self) -> Vec<MicClient> {
        mic_clients()
    }

    fn other_app_playing(&self) -> bool {
        other_app_playing()
    }
}

enum Order {
    Toggle,
    Settings {
        auto_detect: bool,
        ranking: Vec<String>,
    },
}

/// The handle to the meetings thread, which ends with this.
pub(crate) struct Meetings(Sender<Order>);

impl Meetings {
    /// Starts the thread, which first repairs what a crash left behind in `folder`.
    pub(crate) fn spawn(
        room: Arc<dyn MeetingRoom>,
        folder: PathBuf,
        settings: &Settings,
        inbox: Sender<Input>,
        on_event: Arc<dyn Fn(Event) + Send + Sync>,
    ) -> Self {
        let (orders, queue) = mpsc::channel();
        let desk = Desk {
            room,
            folder,
            ranking: settings.general.microphones.clone(),
            detector: MeetingDetector::new(settings.meetings.auto_detect),
            next_detect: Instant::now(),
            inbox,
            on_event,
            recording: None,
        };
        thread::Builder::new()
            .name("wf-meetings".into())
            .spawn(move || desk.run(queue))
            .expect("the meetings thread starts");
        Self(orders)
    }

    pub(crate) fn toggle(&self) {
        let _ = self.0.send(Order::Toggle);
    }

    pub(crate) fn settings_changed(&self, settings: &Settings) {
        let _ = self.0.send(Order::Settings {
            auto_detect: settings.meetings.auto_detect,
            ranking: settings.general.microphones.clone(),
        });
    }
}

struct Recording {
    tape: Box<dyn MeetingTape>,
    path: PathBuf,
    meter: LevelMeter,
    silent_frames: u32,
    heard_system_audio: bool,
    warned: bool,
}

struct Desk {
    room: Arc<dyn MeetingRoom>,
    folder: PathBuf,
    ranking: Vec<String>,
    detector: MeetingDetector,
    next_detect: Instant,
    inbox: Sender<Input>,
    on_event: Arc<dyn Fn(Event) + Send + Sync>,
    recording: Option<Recording>,
}

impl Desk {
    fn run(mut self, orders: Receiver<Order>) {
        repair_leftovers(&self.folder);
        loop {
            let order = match self.wait() {
                Some(wait) => orders.recv_timeout(wait),
                None => orders.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match order {
                Ok(Order::Toggle) => self.toggle(),
                Ok(Order::Settings {
                    auto_detect,
                    ranking,
                }) => {
                    self.detector.set_enabled(auto_detect);
                    self.ranking = ranking;
                }
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.read_events();
            self.detect();
        }
        self.stop();
    }

    fn wait(&self) -> Option<Duration> {
        if self.recording.is_some() {
            return Some(EVENT_POLL);
        }
        self.detector
            .is_enabled()
            .then(|| self.next_detect.saturating_duration_since(Instant::now()))
    }

    fn toggle(&mut self) {
        if self.recording.is_some() {
            self.stop();
        } else {
            self.start();
        }
    }

    fn start(&mut self) {
        if let Err(error) = fs::create_dir_all(&self.folder) {
            return self.report(format!("Couldn't start the meeting notes: {error}"));
        }
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        let path = self.folder.join(format!("{stamp}.{RECORDING_EXTENSION}"));
        match self.room.record(path.clone(), self.ranking.clone()) {
            Ok(tape) => {
                self.recording = Some(Recording {
                    tape,
                    path,
                    meter: LevelMeter::new(),
                    silent_frames: 0,
                    heard_system_audio: false,
                    warned: false,
                });
                let _ = self.inbox.send(Input::Meeting(true));
            }
            Err(error) => self.report(format!("Couldn't start the meeting notes: {error}")),
        }
    }

    fn stop(&mut self) {
        let Some(recording) = self.recording.take() else {
            return;
        };
        let _ = self.inbox.send(Input::Meeting(false));
        let finished = recording.tape.stop().and_then(|()| {
            let target = recording.path.with_extension(FINISHED_EXTENSION);
            fs::rename(&recording.path, target).map_err(|error| error.to_string())
        });
        if let Err(error) = finished {
            self.report(format!("The meeting notes didn't finish cleanly: {error}"));
        }
    }

    fn report(&self, problem: String) {
        let _ = self.inbox.send(Input::Problem(problem));
    }

    fn read_events(&mut self) {
        let Some(recording) = &mut self.recording else {
            return;
        };
        let events: Vec<MeetingEvent> = recording.tape.events().try_iter().collect();
        let mut failure = None;
        let mut silent = false;
        for event in events {
            match event {
                MeetingEvent::Frame {
                    channel: Channel::You,
                    samples,
                } => {
                    if let Some(level) = recording.meter.push(&samples) {
                        (self.on_event)(Event::Level(level));
                    }
                }
                MeetingEvent::Frame {
                    channel: Channel::Others,
                    samples,
                } => {
                    if samples.iter().any(|&sample| sample != 0.0) {
                        recording.heard_system_audio = true;
                    } else if !recording.heard_system_audio {
                        recording.silent_frames += 1;
                        silent |= recording.silent_frames >= SILENT_FRAMES && !recording.warned;
                    }
                }
                MeetingEvent::Device { .. } => {}
                MeetingEvent::Failed(reason) => failure = Some(reason),
            }
        }
        // The tap gives no error when denied, so the only sign is silence while sound plays.
        if silent && self.room.other_app_playing() {
            recording.warned = true;
            self.report(SILENT_SYSTEM_AUDIO.to_owned());
        }
        if let Some(reason) = failure {
            self.report(format!("The meeting recording stopped: {reason}"));
            self.stop();
        }
    }

    fn detect(&mut self) {
        if !self.detector.is_enabled() || Instant::now() < self.next_detect {
            return;
        }
        self.next_detect = Instant::now() + DETECT_POLL;
        let clients = self.room.mic_clients();
        if let Some(app) = self.detector.observe(&clients, self.recording.is_some()) {
            (self.on_event)(Event::SuggestMeeting(app.label().to_owned()));
        }
    }
}

/// Finishes the recordings a crash left in `folder`. A file that can't be repaired stays, for the
/// user to deal with.
fn repair_leftovers(folder: &Path) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_none_or(|extension| extension != RECORDING_EXTENSION)
        {
            continue;
        }
        let repaired = repair_meeting(&path)
            .map_err(|error| error.to_string())
            .and_then(|_| {
                fs::rename(&path, path.with_extension(FINISHED_EXTENSION))
                    .map_err(|error| error.to_string())
            });
        if let Err(error) = repaired {
            eprintln!("could not repair {}: {error}", path.display());
        }
    }
}
