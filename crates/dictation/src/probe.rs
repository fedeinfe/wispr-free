//! Stand-ins for the mic and the focused app, for checking the real engines on recorded audio.
//! Nothing here touches the OS's audio or the keyboard, so a probe can run next to someone working.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use wf_audio::CaptureEvent;
use wf_core::audio::{AudioClip, FRAME_MS, FRAME_SAMPLES};
use wf_platform::context::{Context, ContextDepth, ContextSource, Focus};
use wf_platform::insert::{InsertError, InsertOutcome, PasteChord};

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

struct ClipStream(Receiver<CaptureEvent>);

impl MicStream for ClipStream {
    fn events(&self) -> &Receiver<CaptureEvent> {
        &self.0
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
            let _ = events;
            thread::park();
        });
        Ok(Box::new(ClipStream(stream)))
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
