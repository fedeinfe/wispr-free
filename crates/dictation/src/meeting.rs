//! Starting and stopping meeting notes, and the suggestion to do so (spec §4). The recording and
//! what happens to it afterwards belong to `wf_meetings`. This is only what the running app has
//! to do around it: a thread that keeps the meeting away from the dictation flow's own work, and
//! engines that load behind the recording, never in front of it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use wf_audio::LevelMeter;
use wf_core::audio::{AudioClip, FRAME_MS};
use wf_core::settings::Settings;
use wf_meetings::{
    Channel, LiveEngines, LiveEvent, LiveOptions, LiveTranscript, Meeting, MeetingConfig,
    MeetingDetector, MeetingEvent, MeetingRecorder, MeetingStore, MicClient, NewPart, SummaryMode,
    TimelineFrame, mic_clients, other_app_playing,
};
use wf_models::{Catalog, ModelStore};
use wf_pipeline::{Asr, AsrOptions, PipelineError, SpeechSegment, Transcript, Vad};

use crate::coordinator::Input;
use crate::engines::{Engines, LlmSlot};
use crate::meeting_work::{self, ChatRun, Ticket, Work, WorkStep};
use crate::plan::{AsrModel, meeting_speech, single_language};
use crate::{Clock, Event};

/// How often a recording's events are read.
const EVENT_POLL: Duration = Duration::from_millis(100);
/// How often the OS is asked who holds the mic, when the user opted in.
const DETECT_POLL: Duration = Duration::from_secs(2);
/// Frames of pure digital silence from the system channel, while something plays, before the
/// silence counts as a denied permission: 5 s.
const SILENT_FRAMES: u32 = 5_000 / FRAME_MS;
/// How long before the maximum length the user hears about it.
const LENGTH_WARNING: Duration = Duration::from_secs(60);

const LENGTH_HINT: &str = "One minute of meeting notes left";

#[cfg(target_os = "macos")]
const SILENT_SYSTEM_AUDIO: &str =
    "System audio is silent. Allow System Audio Recording in Privacy & Security";
#[cfg(not(target_os = "macos"))]
const SILENT_SYSTEM_AUDIO: &str = "System audio is silent";

/// A meeting being written to disk.
pub trait MeetingTape: Send {
    fn events(&self) -> &Receiver<MeetingEvent>;
    /// Finishes the file and returns the recording's length.
    fn stop(self: Box<Self>) -> Result<Duration, String>;
}

impl MeetingTape for MeetingRecorder {
    fn events(&self) -> &Receiver<MeetingEvent> {
        MeetingRecorder::events(self)
    }

    fn stop(self: Box<Self>) -> Result<Duration, String> {
        MeetingRecorder::stop(*self).map_err(|error| error.to_string())
    }
}

/// The OS, as far as meetings go.
pub trait MeetingRoom: Send + Sync {
    /// Starts recording to `path`. `timeline` gets every frame as it goes to the file.
    fn record(
        &self,
        path: PathBuf,
        ranking: Vec<String>,
        timeline: Option<SyncSender<TimelineFrame>>,
    ) -> Result<Box<dyn MeetingTape>, String>;
    fn mic_clients(&self) -> Vec<MicClient>;
    fn other_app_playing(&self) -> bool;
}

pub struct SystemMeetingRoom;

impl MeetingRoom for SystemMeetingRoom {
    fn record(
        &self,
        path: PathBuf,
        ranking: Vec<String>,
        timeline: Option<SyncSender<TimelineFrame>>,
    ) -> Result<Box<dyn MeetingTape>, String> {
        let config = MeetingConfig {
            mic_ranking: ranking,
            timeline,
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

/// What's going on with meetings, for whoever comes to look.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MeetingsStatus {
    /// The meeting being recorded.
    pub recording: Option<String>,
    /// Meetings with work running or waiting, in order.
    pub work: Vec<(String, WorkStep)>,
}

/// What the meetings thread, the work thread and the app's handle all reach.
pub(crate) struct Office {
    pub(crate) store: MeetingStore,
    settings: Mutex<Settings>,
    pub(crate) catalog: Arc<Catalog>,
    default_models: PathBuf,
    pub(crate) engines: Arc<dyn Engines>,
    pub(crate) llm: Arc<LlmSlot>,
    clock: Arc<dyn Clock>,
    inbox: Sender<Input>,
    pub(crate) on_event: Arc<dyn Fn(Event) + Send + Sync>,
    status: Mutex<MeetingsStatus>,
    tickets: Mutex<Vec<Ticket>>,
    chat_runs: AtomicU64,
}

impl Office {
    #[expect(clippy::too_many_arguments, reason = "the parts of one struct")]
    pub(crate) fn new(
        root: PathBuf,
        settings: Settings,
        catalog: Arc<Catalog>,
        default_models: PathBuf,
        engines: Arc<dyn Engines>,
        llm: Arc<LlmSlot>,
        clock: Arc<dyn Clock>,
        inbox: Sender<Input>,
        on_event: Arc<dyn Fn(Event) + Send + Sync>,
    ) -> Self {
        Self {
            store: MeetingStore::new(root),
            settings: Mutex::new(settings),
            catalog,
            default_models,
            engines,
            llm,
            clock,
            inbox,
            on_event,
            status: Mutex::default(),
            tickets: Mutex::default(),
            chat_runs: AtomicU64::new(0),
        }
    }

    pub(crate) fn settings(&self) -> Settings {
        self.settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn models(&self, settings: &Settings) -> ModelStore {
        ModelStore::configured(
            settings.pipeline.models_folder.as_deref(),
            &self.default_models,
        )
    }

    pub(crate) fn status(&self) -> MutexGuard<'_, MeetingsStatus> {
        self.status.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn tickets(&self) -> MutexGuard<'_, Vec<Ticket>> {
        self.tickets.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn next_chat_run(&self) -> u64 {
        self.chat_runs.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub(crate) fn news(&self, news: MeetingNews) {
        (self.on_event)(Event::Meetings(news));
    }

    pub(crate) fn report(&self, problem: String) {
        let _ = self.inbox.send(Input::Problem(problem));
    }

    pub(crate) fn toast(&self, text: impl Into<String>) {
        (self.on_event)(Event::Toast(text.into()));
    }
}

/// What the Meetings window hears about.
#[derive(Debug, Clone, PartialEq)]
pub enum MeetingNews {
    /// Meetings appeared on disk without anyone recording them: repaired or brought in at launch.
    ListChanged,
    Recording(String),
    Stopped(String),
    /// A line of the live transcript, or trouble with it.
    Live {
        id: String,
        event: LiveEvent,
    },
    /// Work on a meeting moved on.
    Working {
        id: String,
        step: WorkStep,
    },
    /// Work on a meeting ended. Each stage's own result is in its record.
    WorkDone {
        id: String,
        problem: Option<String>,
    },
    Chat {
        run: u64,
        update: ChatUpdate,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChatUpdate {
    Token(String),
    /// Saved with the meeting. The error is written for the chat panel.
    Done(Result<wf_meetings::ChatTurn, String>),
}

enum Order {
    Toggle,
    Resume(String),
    Settings(Box<Settings>),
    Close(Sender<()>),
}

/// The way in to the meetings thread, which ends when the last of these goes.
#[derive(Clone)]
pub(crate) struct Meetings {
    orders: Sender<Order>,
    work: Sender<Work>,
    office: Arc<Office>,
}

impl Meetings {
    /// Starts the threads. The first thing they do is bring in what a crash or an older build
    /// left behind.
    pub(crate) fn spawn(room: Arc<dyn MeetingRoom>, office: Arc<Office>) -> Self {
        let (orders, queue) = mpsc::channel();
        let (work, jobs) = mpsc::channel();
        let settings = office.settings();
        let desk = Desk {
            room,
            office: Arc::clone(&office),
            work: work.clone(),
            ranking: settings.general.microphones.clone(),
            detector: MeetingDetector::new(settings.meetings.auto_detect),
            next_detect: Instant::now(),
            recording: None,
        };
        thread::Builder::new()
            .name("wf-meetings".into())
            .spawn(move || desk.run(queue))
            .expect("the meetings thread starts");
        let worker = Arc::clone(&office);
        thread::Builder::new()
            .name("wf-meeting-work".into())
            .spawn(move || meeting_work::run(&worker, jobs))
            .expect("the meeting work thread starts");
        Self {
            orders,
            work,
            office,
        }
    }

    pub(crate) fn toggle(&self) {
        let _ = self.orders.send(Order::Toggle);
    }

    pub(crate) fn resume(&self, id: &str) {
        let _ = self.orders.send(Order::Resume(id.to_owned()));
    }

    pub(crate) fn close(&self, done: Sender<()>) {
        let _ = self.orders.send(Order::Close(done));
    }

    pub(crate) fn settings_changed(&self, settings: &Settings) {
        *self
            .office
            .settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = settings.clone();
        let _ = self
            .orders
            .send(Order::Settings(Box::new(settings.clone())));
    }

    pub(crate) fn status(&self) -> MeetingsStatus {
        self.office.status().clone()
    }

    pub(crate) fn finish_stages(&self, id: &str) {
        self.office.finish_stages(&self.work, id);
    }

    pub(crate) fn write_summary(&self, id: &str, mode: SummaryMode) {
        self.office.write_summary(&self.work, id, mode);
    }

    pub(crate) fn cancel_work(&self, id: &str) {
        self.office.cancel_work(id);
    }

    pub(crate) fn ask(&self, id: &str, question: &str) -> ChatRun {
        self.office.ask(id, question)
    }
}

/// What the pill's bars follow in a meeting: whoever is louder, so they move when the others talk.
#[derive(Default)]
struct CallLevels {
    you: (LevelMeter, f32),
    others: (LevelMeter, f32),
}

impl CallLevels {
    fn you(&mut self, samples: &[f32]) -> Option<f32> {
        self.you.1 = self.you.0.push(samples)?;
        Some(self.you.1.max(self.others.1))
    }

    fn others(&mut self, samples: &[f32]) -> Option<f32> {
        self.others.1 = self.others.0.push(samples)?;
        Some(self.you.1.max(self.others.1))
    }
}

struct Recording {
    tape: Box<dyn MeetingTape>,
    meeting: Meeting,
    live: Option<LiveTranscript>,
    levels: CallLevels,
    silent_frames: u32,
    heard_system_audio: bool,
    warned: bool,
    started: Duration,
    length_warned: bool,
}

struct Desk {
    room: Arc<dyn MeetingRoom>,
    office: Arc<Office>,
    work: Sender<Work>,
    ranking: Vec<String>,
    detector: MeetingDetector,
    next_detect: Instant,
    recording: Option<Recording>,
}

impl Desk {
    fn run(mut self, orders: Receiver<Order>) {
        self.bring_in_leftovers();
        loop {
            let order = match self.wait() {
                Some(wait) => orders.recv_timeout(wait),
                None => orders.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match order {
                Ok(Order::Toggle) => self.toggle(),
                Ok(Order::Resume(id)) => self.resume(id),
                Ok(Order::Settings(settings)) => {
                    self.detector.set_enabled(settings.meetings.auto_detect);
                    self.ranking.clone_from(&settings.general.microphones);
                }
                Ok(Order::Close(done)) => self.close(Some(done)),
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.read_events();
            self.watch_the_clock();
            self.detect();
        }
        self.close(None);
    }

    fn bring_in_leftovers(&self) {
        let store = &self.office.store;
        let adopted = store.adopt_flat_recordings();
        let recovered = store.recover();
        for result in [&adopted, &recovered] {
            if let Err(error) = result {
                eprintln!("meetings left behind can't be read: {error}");
            }
        }
        if [adopted, recovered]
            .iter()
            .any(|result| result.as_ref().is_ok_and(|ids| !ids.is_empty()))
        {
            self.office.news(MeetingNews::ListChanged);
        }
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
            match self.office.store.create() {
                Ok(meeting) => self.start(meeting, true),
                Err(error) => self.cannot_start(&error),
            }
        }
    }

    fn resume(&mut self, id: String) {
        if self.recording.is_some() {
            return self
                .office
                .report("A meeting is already recording".to_owned());
        }
        if self
            .office
            .status()
            .work
            .iter()
            .any(|(busy, _)| *busy == id)
        {
            return self
                .office
                .report("That meeting is still being processed".to_owned());
        }
        match self.office.store.open(&id) {
            Ok(meeting) => self.start(meeting, false),
            Err(error) => self.cannot_start(&error),
        }
    }

    fn cannot_start(&self, error: &dyn std::fmt::Display) {
        self.office
            .report(format!("Couldn't start the meeting notes: {error}"));
    }

    fn start(&mut self, mut meeting: Meeting, new: bool) {
        let part = match meeting.begin_part() {
            Ok(part) => part,
            Err(error) => return self.cannot_start(&error),
        };
        let live = self.start_live(&meeting, &part);
        let timeline = live.as_ref().map(LiveTranscript::feed);
        let id = meeting.record().id.clone();
        match self.room.record(part.path, self.ranking.clone(), timeline) {
            Ok(tape) => {
                self.recording = Some(Recording {
                    tape,
                    meeting,
                    live,
                    levels: CallLevels::default(),
                    silent_frames: 0,
                    heard_system_audio: false,
                    warned: false,
                    started: self.office.clock.now(),
                    length_warned: false,
                });
                self.office.status().recording = Some(id.clone());
                let _ = self.office.inbox.send(Input::Meeting(true));
                self.office.news(MeetingNews::Recording(id));
            }
            Err(error) => {
                if let Err(error) = meeting.abandon_part() {
                    eprintln!("meeting {id} can't take its recording back: {error}");
                }
                if new && let Err(error) = self.office.store.delete(&id) {
                    eprintln!("meeting {id} can't be removed: {error}");
                }
                self.cannot_start(&error);
            }
        }
    }

    /// The live transcript for a new part. The recording doesn't depend on it: without it the
    /// meeting records just the same, and the refine pass makes the transcript.
    fn start_live(&self, meeting: &Meeting, part: &NewPart) -> Option<LiveTranscript> {
        let settings = self.office.settings();
        if !settings.meetings.live_transcript {
            return None;
        }
        let models = self.office.models(&settings);
        let speech = meeting_speech(&settings, &self.office.catalog, &models);
        let (asr, vad) = match speech {
            Ok(models) => models,
            Err(problem) => {
                self.office.report(format!("No live transcript: {problem}"));
                return None;
            }
        };
        let engines = self.office.engines.scope();
        let lazy_vad = || -> Box<dyn Vad> {
            Box::new(LazyVad {
                engines: Arc::clone(&engines),
                model: vad.clone(),
                engine: None,
            })
        };
        let live_engines = LiveEngines {
            asr: Box::new(LazyAsr {
                engines: Arc::clone(&engines),
                model: asr,
                engine: None,
            }),
            you_vad: lazy_vad(),
            others_vad: lazy_vad(),
        };
        let options = LiveOptions {
            asr: AsrOptions {
                language: single_language(&settings),
                bias_words: settings.style.dictionary.clone(),
            },
            ..LiveOptions::default()
        };
        match LiveTranscript::start(meeting, part, live_engines, options) {
            Ok(live) => Some(live),
            Err(error) => {
                self.office.report(format!("No live transcript: {error}"));
                None
            }
        }
    }

    fn stop(&mut self) {
        let Some(mut recording) = self.recording.take() else {
            return;
        };
        self.read_live(&mut recording);
        let id = recording.meeting.record().id.clone();
        self.office.status().recording = None;
        let _ = self.office.inbox.send(Input::Meeting(false));
        self.office.news(MeetingNews::Stopped(id.clone()));
        let length = recording.tape.stop().map_err(|error| {
            self.office
                .report(format!("The meeting notes didn't finish cleanly: {error}"));
        });
        self.office
            .close_meeting(&self.work, recording.meeting, recording.live, length.ok());
    }

    /// For when the app goes: what's on disk is finished properly, and nothing new starts. If time
    /// runs out before the backlog is written, the next launch recovers the meeting from its file.
    fn close(&mut self, done: Option<Sender<()>>) {
        if let Some(recording) = self.recording.take() {
            let _ = self.office.inbox.send(Input::Meeting(false));
            let Recording {
                tape,
                mut meeting,
                live,
                ..
            } = recording;
            let length = tape.stop();
            let closing = move || {
                if let Some(live) = live {
                    let _ = live.finish();
                }
                let closed = match length {
                    Ok(length) => meeting.end_part(length),
                    Err(_) => meeting.recover_part(),
                };
                if let Err(error) = closed {
                    eprintln!("the meeting can't be closed: {error}");
                }
                if let Some(done) = done {
                    let _ = done.send(());
                }
            };
            thread::spawn(closing);
        } else if let Some(done) = done {
            let _ = done.send(());
        }
    }

    fn read_events(&mut self) {
        let Some(mut recording) = self.recording.take() else {
            return;
        };
        self.read_live(&mut recording);
        let events: Vec<MeetingEvent> = recording.tape.events().try_iter().collect();
        let mut failure = None;
        let mut silent = false;
        for event in events {
            match event {
                MeetingEvent::Frame {
                    channel: Channel::You,
                    samples,
                } => {
                    if let Some(level) = recording.levels.you(&samples) {
                        (self.office.on_event)(Event::Level(level));
                    }
                }
                MeetingEvent::Frame {
                    channel: Channel::Others,
                    samples,
                } => {
                    if let Some(level) = recording.levels.others(&samples) {
                        (self.office.on_event)(Event::Level(level));
                    }
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
            self.office.report(SILENT_SYSTEM_AUDIO.to_owned());
        }
        self.recording = Some(recording);
        if let Some(reason) = failure {
            self.office
                .report(format!("The meeting recording stopped: {reason}"));
            self.stop();
        }
    }

    fn read_live(&self, recording: &mut Recording) {
        let Some(live) = &recording.live else {
            return;
        };
        let id = &recording.meeting.record().id;
        for event in live.events().try_iter() {
            if let LiveEvent::Failed(reason) = &event {
                self.office
                    .report(format!("The live transcript stopped: {reason}"));
            }
            self.office.news(MeetingNews::Live {
                id: id.clone(),
                event,
            });
        }
    }

    /// Enforces the maximum length, which is there so a forgotten recording doesn't fill the disk.
    fn watch_the_clock(&mut self) {
        let Some(recording) = &mut self.recording else {
            return;
        };
        let minutes = self.office.settings().meetings.max_length_minutes;
        let limit = Duration::from_secs(u64::from(minutes) * 60);
        let elapsed = self.office.clock.now().saturating_sub(recording.started);
        if elapsed >= limit {
            self.office
                .toast(format!("Meeting notes stopped at {minutes} minutes"));
            self.stop();
        } else if elapsed + LENGTH_WARNING >= limit && !recording.length_warned {
            recording.length_warned = true;
            self.office.toast(LENGTH_HINT);
        }
    }

    fn detect(&mut self) {
        if !self.detector.is_enabled() || Instant::now() < self.next_detect {
            return;
        }
        self.next_detect = Instant::now() + DETECT_POLL;
        let clients = self.room.mic_clients();
        if let Some(app) = self.detector.observe(&clients, self.recording.is_some()) {
            (self.office.on_event)(Event::SuggestMeeting(app.label().to_owned()));
        }
    }
}

/// An engine that starts on first use, so that opening a recording waits for no model. Frames wait
/// in the live transcript's queue meanwhile.
struct LazyAsr {
    engines: Arc<dyn Engines>,
    model: AsrModel,
    engine: Option<Box<dyn Asr>>,
}

impl Asr for LazyAsr {
    fn transcribe(
        &mut self,
        clip: &AudioClip,
        options: &AsrOptions,
    ) -> Result<Transcript, PipelineError> {
        if self.engine.is_none() {
            self.engine = Some(self.engines.asr(&self.model)?);
        }
        let result = self
            .engine
            .as_mut()
            .expect("just loaded")
            .transcribe(clip, options);
        if result.is_err() {
            // A dead host stays dead, and the next chunk can start a new one.
            self.engine = None;
        }
        result
    }
}

struct LazyVad {
    engines: Arc<dyn Engines>,
    model: PathBuf,
    engine: Option<Box<dyn Vad>>,
}

impl Vad for LazyVad {
    fn accept(&mut self, samples: &[f32]) -> Result<Vec<SpeechSegment>, PipelineError> {
        if self.engine.is_none() {
            self.engine = Some(self.engines.vad(&self.model)?);
        }
        self.engine.as_mut().expect("just loaded").accept(samples)
    }

    fn is_speaking(&self) -> bool {
        self.engine.as_ref().is_some_and(|vad| vad.is_speaking())
    }

    fn finish(&mut self) -> Result<Vec<SpeechSegment>, PipelineError> {
        match &mut self.engine {
            Some(vad) => vad.finish(),
            None => Ok(Vec::new()),
        }
    }
}
