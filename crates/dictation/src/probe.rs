//! Stand-ins for the mic and the focused app, for checking the real engines on recorded audio.
//! Nothing here touches the OS's audio or the keyboard, so a probe can run next to someone working.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use wf_audio::CaptureEvent;
use wf_core::audio::{AudioClip, FRAME_MS, FRAME_SAMPLES};
use wf_meetings::{Error, MeetingEvent, MeetingWriter, MicClient, StereoFrame, TimelineFrame};
use wf_platform::context::{Context, ContextDepth, ContextSource, Focus};
use wf_platform::insert::{InsertError, InsertOutcome, PasteChord};
use wf_platform::media::{MediaControl, MediaError, Output};

use crate::meeting::{MeetingRoom, MeetingTape};
use crate::{Insertion, MicStream, Microphone};

/// Reads a 16-bit mono WAV at the pipeline's rate.
pub fn read_wav(path: &Path) -> Result<AudioClip, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().expect("four bytes"));
        let size = size as usize;
        if &bytes[at..at + 4] == b"data" {
            let end = (at + 8 + size).min(bytes.len());
            let samples = bytes[at + 8..end]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&pair| f32::from(i16::from_le_bytes(pair)) / 32_768.0)
                .collect();
            return Ok(AudioClip::new(samples));
        }
        at += 8 + size + size % 2;
    }
    Err(format!("{}: no audio", path.display()))
}

/// Plays a clip as if someone said it into the mic.
pub struct ClipMicrophone {
    clip: Mutex<AudioClip>,
    /// Sends frames at the speed they would arrive, rather than all at once.
    real_time: bool,
    played: Arc<AtomicBool>,
}

impl ClipMicrophone {
    pub fn new(clip: AudioClip, real_time: bool) -> Self {
        Self {
            clip: Mutex::new(clip),
            real_time,
            played: Arc::default(),
        }
    }

    pub fn set_clip(&self, clip: AudioClip) {
        *self.clip.lock().unwrap_or_else(PoisonError::into_inner) = clip;
        self.played.store(false, Ordering::SeqCst);
    }

    /// Whether the clip has played out.
    pub fn played(&self) -> bool {
        self.played.load(Ordering::SeqCst)
    }
}

struct ClipStream {
    events: Receiver<CaptureEvent>,
    /// Dropping this ends the thread that played the clip.
    _open: Sender<()>,
}

impl MicStream for ClipStream {
    fn events(&self) -> &Receiver<CaptureEvent> {
        &self.events
    }
}

impl Microphone for ClipMicrophone {
    fn problem(&self) -> Option<String> {
        None
    }

    fn open(&self, _ranking: Vec<String>) -> Result<Box<dyn MicStream>, String> {
        let clip = self
            .clip
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let (events, stream) = mpsc::channel();
        let (open, closed) = mpsc::channel();
        let (real_time, played) = (self.real_time, Arc::clone(&self.played));
        thread::spawn(move || {
            for frame in clip.samples().chunks(FRAME_SAMPLES) {
                let mut frame = frame.to_vec();
                frame.resize(FRAME_SAMPLES, 0.0);
                if events.send(CaptureEvent::Frame(frame)).is_err() {
                    return;
                }
                if real_time {
                    thread::sleep(Duration::from_millis(u64::from(FRAME_MS)));
                }
            }
            played.store(true, Ordering::SeqCst);
            // Stays open like a real mic until the flow closes it.
            let _ = closed.recv();
        });
        Ok(Box::new(ClipStream {
            events: stream,
            _open: open,
        }))
    }
}

/// Plays two recorded channels as if a meeting were going on, through the same file and timeline
/// the real recorder uses. Opens no device, so a meeting can run next to someone working.
pub struct ClipRoom {
    you: Arc<AudioClip>,
    others: Arc<AudioClip>,
    /// How many times faster than a real meeting.
    pace: f32,
    played: Arc<AtomicBool>,
}

impl ClipRoom {
    pub fn new(you: AudioClip, others: AudioClip, pace: f32) -> Self {
        Self {
            you: Arc::new(you),
            others: Arc::new(others),
            pace,
            played: Arc::default(),
        }
    }

    /// Whether the clips have played out. The tape stays open like a real recording until stopped.
    pub fn played(&self) -> bool {
        self.played.load(Ordering::SeqCst)
    }
}

impl MeetingRoom for ClipRoom {
    fn record(
        &self,
        path: PathBuf,
        _ranking: Vec<String>,
        timeline: Option<SyncSender<TimelineFrame>>,
    ) -> Result<Box<dyn MeetingTape>, String> {
        let mut writer = MeetingWriter::create(&path).map_err(|error| error.to_string())?;
        let (you, others) = (Arc::clone(&self.you), Arc::clone(&self.others));
        let (pace, played) = (self.pace, Arc::clone(&self.played));
        played.store(false, Ordering::SeqCst);
        let (stop, stopped) = mpsc::channel();
        let (_events_tx, events) = mpsc::channel();
        let thread = thread::spawn(move || -> Result<Duration, Error> {
            let frames = you.samples().len().max(others.samples().len()) / FRAME_SAMPLES;
            let started = Instant::now();
            for index in 0..frames {
                let frame = StereoFrame {
                    you: frame_at(&you, index),
                    others: frame_at(&others, index),
                };
                writer.write_frame(&frame.you, &frame.others)?;
                if let Some(timeline) = &timeline {
                    let index = index as u64;
                    let _ = timeline.try_send(TimelineFrame { index, frame });
                }
                let due = Duration::from_millis(index as u64 * u64::from(FRAME_MS)).div_f32(pace);
                thread::sleep(due.saturating_sub(started.elapsed()));
                if stopped.try_recv().is_ok() {
                    drop(timeline);
                    return writer.finish();
                }
            }
            played.store(true, Ordering::SeqCst);
            drop(timeline);
            let _ = stopped.recv();
            writer.finish()
        });
        Ok(Box::new(ClipTape {
            events,
            stop,
            thread,
            _feed: _events_tx,
        }))
    }

    fn mic_clients(&self) -> Vec<MicClient> {
        Vec::new()
    }

    fn other_app_playing(&self) -> bool {
        false
    }
}

fn frame_at(clip: &AudioClip, index: usize) -> Vec<f32> {
    let samples = clip.samples();
    let start = (index * FRAME_SAMPLES).min(samples.len());
    let mut frame = samples[start..(start + FRAME_SAMPLES).min(samples.len())].to_vec();
    frame.resize(FRAME_SAMPLES, 0.0);
    frame
}

struct ClipTape {
    events: Receiver<MeetingEvent>,
    stop: Sender<()>,
    thread: JoinHandle<Result<Duration, Error>>,
    /// Keeps `events` open and quiet.
    _feed: Sender<MeetingEvent>,
}

impl MeetingTape for ClipTape {
    fn events(&self) -> &Receiver<MeetingEvent> {
        &self.events
    }

    fn stop(self: Box<Self>) -> Result<Duration, String> {
        let _ = self.stop.send(());
        match self.thread.join() {
            Ok(result) => result.map_err(|error| error.to_string()),
            Err(_) => Err("the clip's thread panicked".to_owned()),
        }
    }
}

/// Notes what would have been typed.
#[derive(Clone, Default)]
pub struct TextLog(Arc<Mutex<Vec<String>>>);

impl TextLog {
    pub fn texts(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Insertion for TextLog {
    fn insert(
        &mut self,
        text: &str,
        on_pasted: &mut dyn FnMut(),
    ) -> Result<InsertOutcome, InsertError> {
        on_pasted();
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(text.to_owned());
        Ok(InsertOutcome::Pasted {
            chord: PasteChord::CmdV,
            clipboard_restored: true,
        })
    }

    fn press_enter(&mut self) -> Result<(), InsertError> {
        Ok(())
    }

    fn copy(&mut self, _text: &str) -> Result<(), InsertError> {
        Ok(())
    }
}

/// Sound the OS is never asked about, so a probe can't mute or pause anything.
pub struct NoMedia;

impl MediaControl for NoMedia {
    fn output(&self) -> Result<Output, MediaError> {
        Err(MediaError::NoOutput)
    }

    fn set_muted(&self, _device: &str, _muted: bool) -> Result<(), MediaError> {
        Err(MediaError::NoOutput)
    }

    fn pause_players(&self) -> Vec<String> {
        Vec::new()
    }

    fn resume_players(&self, _players: &[String]) {}

    fn ask_players(&self) {}
}

/// A focused text field the OS is never asked about.
pub struct NoContext;

impl ContextSource for NoContext {
    fn read(&self, _depth: ContextDepth) -> Context {
        Context {
            app: None,
            window_title: None,
            focus: Focus::Editable,
            text: None,
        }
    }
}
