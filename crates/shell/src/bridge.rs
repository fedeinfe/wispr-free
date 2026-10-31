//! What Swift sees (spec, Bridge). One object, [`Shell`], with no window in it: it holds the flow
//! and the settings, takes every path from its caller and reports through one [`EventSink`].
//!
//! Every call returns a `Result`, because a panic in a call that can't throw is fatal in Swift.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::Duration;

use wf_core::dictation::PillInput;
use wf_core::pill::PillState;
use wf_core::settings::{Settings, SettingsStore};
use wf_dictation::{Dictation, Event};

use crate::flow;
use crate::memory;
use crate::paths::{self, Paths};

/// How long quitting waits for a running recording to be closed. If it isn't done by then, the
/// next launch repairs the meeting.
const MEETING_CLOSE_LIMIT: Duration = Duration::from_secs(5);
/// How long quitting waits for the sound to come back. A mute that outlasts this comes off at the
/// next launch.
const MEDIA_CLOSE_LIMIT: Duration = Duration::from_secs(3);

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum ShellError {
    #[error("no home folder to keep data in")]
    NoHome,
    #[error("{path} is the user's own data")]
    RealData { path: String },
    #[error("{path} isn't an absolute path")]
    UnclearPath { path: String },
    #[error("Couldn't save settings: {reason}")]
    Disk { reason: String },
}

/// The settings as of one moment. Saving from an older one fails with the newer one.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Versioned {
    pub settings: Settings,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SaveOutcome {
    Saved {
        current: Versioned,
    },
    /// Something else saved first. Nothing changed: apply the edit to `current` and save again.
    Stale {
        current: Versioned,
    },
}

/// What the app gets told. Delivered on the flow's threads.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum ShellEvent {
    Pill { state: PillState },
    Level { value: f32 },
    Toast { text: String },
    Recent { transcripts: Vec<String> },
    Meeting { active: bool },
    SuggestMeeting { app: String },
    NoTextField { text: String },
    SettingsChanged { current: Versioned },
}

/// Runs on a thread of the flow's, so it must not wait: only hand the event on.
#[uniffi::export(with_foreign)]
pub trait EventSink: Send + Sync {
    fn on_event(&self, event: ShellEvent);
}

/// The settings with the revision that makes compare and swap work. The lock also keeps events in
/// the order of the saves they report.
struct Board {
    store: SettingsStore,
    revision: Mutex<u64>,
    sink: Arc<dyn EventSink>,
    dictation: OnceLock<Dictation>,
}

impl Board {
    fn lock_revision(&self) -> MutexGuard<'_, u64> {
        self.revision.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn current(&self, revision: u64) -> Versioned {
        Versioned {
            settings: self.store.get(),
            revision,
        }
    }

    fn save(&self, settings: Settings, based_on: u64) -> Result<SaveOutcome, ShellError> {
        let mut revision = self.lock_revision();
        if based_on != *revision {
            return Ok(SaveOutcome::Stale {
                current: self.current(*revision),
            });
        }
        let written = self.store.update(|current| *current = settings);
        let current = self.applied(&mut revision);
        written.map_err(|error| ShellError::Disk {
            reason: error.to_string(),
        })?;
        Ok(SaveOutcome::Saved { current })
    }

    /// A change from inside Rust, on the latest settings, so no save can undo it.
    fn change(&self, change: impl FnOnce(&mut Settings)) {
        let mut revision = self.lock_revision();
        let written = self.store.update(change);
        self.applied(&mut revision);
        if let Err(error) = written {
            self.sink.on_event(ShellEvent::Toast {
                text: format!("Couldn't save settings: {error}"),
            });
        }
    }

    /// Every save holds for this run, even one the disk refused.
    fn applied(&self, revision: &mut u64) -> Versioned {
        *revision += 1;
        let current = self.current(*revision);
        if let Some(dictation) = self.dictation.get() {
            dictation.settings_changed(current.settings.clone());
        }
        self.sink.on_event(ShellEvent::SettingsChanged {
            current: current.clone(),
        });
        current
    }

    /// The flow found a word for the dictionary. The toast is how the user finds out.
    fn learn_word(&self, word: &str) {
        let mut added = false;
        self.change(|settings| added = settings.style.add_word(word));
        if added {
            self.sink.on_event(ShellEvent::Toast {
                text: format!("Added “{word}” to the dictionary"),
            });
        }
    }

    fn deliver(&self, event: Event) {
        let event = match event {
            Event::Pill(state) => ShellEvent::Pill { state },
            Event::Level(value) => ShellEvent::Level { value },
            Event::Toast(text) => ShellEvent::Toast { text },
            Event::Recent(transcripts) => ShellEvent::Recent { transcripts },
            Event::Meeting(active) => ShellEvent::Meeting { active },
            Event::SuggestMeeting(app) => ShellEvent::SuggestMeeting { app },
            Event::NoTextField(text) => ShellEvent::NoTextField { text },
            Event::WordLearned(word) => return self.learn_word(&word),
            Event::Unloaded => return memory::release_freed(),
            // Their windows bring them across: Onboarding the trials, Meetings its news.
            Event::Trial(_)
            | Event::TrialFailed { .. }
            | Event::Inserted(_)
            | Event::Meetings(_) => return,
        };
        self.sink.on_event(event);
    }
}

/// The running app, behind everything Swift draws.
#[derive(uniffi::Object)]
pub struct Shell {
    board: Arc<Board>,
    dictation: Dictation,
}

impl Shell {
    /// Starts on `store`, with `start_flow` making the flow. Events reach the sink from then on.
    pub fn wired(
        store: SettingsStore,
        sink: Arc<dyn EventSink>,
        start_flow: impl FnOnce(Settings, flow::OnEvent) -> Dictation,
    ) -> Arc<Self> {
        let board = Arc::new(Board {
            store,
            revision: Mutex::new(0),
            sink,
            dictation: OnceLock::new(),
        });
        // Weak, or the flow would keep the board alive and the board the flow.
        let events = Arc::downgrade(&board);
        let dictation = start_flow(
            board.store.get(),
            Arc::new(move |event| {
                if let Some(board) = Weak::upgrade(&events) {
                    board.deliver(event);
                }
            }),
        );
        let _ = board.dictation.set(dictation.clone());
        Arc::new(Self { board, dictation })
    }
}

#[uniffi::export]
impl Shell {
    /// Nothing listens to the keys or opens a microphone here, so a dev app that's only been
    /// started can't race the real one.
    #[uniffi::constructor]
    pub async fn start(paths: Paths, sink: Arc<dyn EventSink>) -> Result<Arc<Self>, ShellError> {
        paths.check(&paths::real_folders())?;
        let store = SettingsStore::open(&paths.settings);
        Ok(Self::wired(store, sink, |settings, on_event| {
            Dictation::start(flow::parts_at(&paths, settings, on_event))
        }))
    }

    /// Puts things back before the app goes away: the sound, then any recording.
    pub async fn quit(&self) -> Result<(), ShellError> {
        self.dictation.close_media(MEDIA_CLOSE_LIMIT);
        self.dictation.close_meetings(MEETING_CLOSE_LIMIT);
        Ok(())
    }

    pub fn on_pill(&self, input: PillInput) -> Result<(), ShellError> {
        self.dictation.on_pill(input);
        Ok(())
    }

    pub fn paste_last(&self) -> Result<(), ShellError> {
        self.dictation.paste_last();
        Ok(())
    }

    pub fn paste_recent(&self, index: u32) -> Result<(), ShellError> {
        self.dictation.paste_recent(index as usize);
        Ok(())
    }

    pub fn copy_text(&self, text: String) -> Result<(), ShellError> {
        self.dictation.copy_text(text);
        Ok(())
    }

    pub fn settings(&self) -> Result<Versioned, ShellError> {
        let revision = self.board.lock_revision();
        Ok(self.board.current(*revision))
    }

    /// Stores `settings` unless something else saved since `revision`.
    pub async fn save(
        self: Arc<Self>,
        settings: Settings,
        revision: u64,
    ) -> Result<SaveOutcome, ShellError> {
        self.board.save(settings, revision)
    }
}
