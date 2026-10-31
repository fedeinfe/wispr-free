//! What Swift sees (spec, Bridge). One object, [`Shell`], with no window in it: it holds the flow
//! and the settings, takes every path from its caller and reports through one [`EventSink`].
//!
//! Every call returns a `Result`, because a panic in a call that can't throw is fatal in Swift.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant};

use wf_audio::{AudioBackend as _, CpalBackend, InputDevice, sounds};
use wf_core::dictation::PillInput;
use wf_core::pill::dock::{Display, Point};
use wf_core::pill::{PillCommand, PillState, Sound};
use wf_core::settings::{Settings, SettingsStore};
use wf_dictation::{Dictation, Event};
use wf_models::Catalog;

use crate::flow::{self, KeySlot, rank_first};
use crate::memory;
use crate::menus::{self, MenuAction, MenuEntry};
use crate::paths::{self, Paths};
use crate::pill::{PillCore, PillFrame, Question, save_prefs};
use crate::prefs::Prefs;
use crate::rehearsal;
use crate::settings::values::choose_tier;

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
// It crosses by value, so a box would save nothing.
#[allow(clippy::large_enum_variant)]
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
    /// A menu asked for a window.
    Open { window: AppWindow },
    /// A menu asked the app to quit. The app calls [`Shell::quit`] on its way out.
    QuitAsked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AppWindow {
    Settings,
    Meetings,
}

/// Something for the pill to show, the way the flow would have asked.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum PillScene {
    State { state: PillState },
    Toast { text: String },
    SuggestMeeting { app: String },
    NoTextField { text: String },
}

fn disk_error(error: impl std::fmt::Display) -> ShellError {
    ShellError::Disk {
        reason: error.to_string(),
    }
}

/// Runs on a thread of the flow's, so it must not wait: only hand the event on.
#[uniffi::export(with_foreign)]
pub trait EventSink: Send + Sync {
    fn on_event(&self, event: ShellEvent);
}

/// What the pill needs from the machine.
pub struct PillParts {
    /// Where the pill was left.
    pub prefs: PathBuf,
    pub play: Box<dyn Fn(Sound) + Send + Sync>,
    /// Takes a moment, so it runs on a thread of its own.
    pub microphones: Box<dyn FnOnce() -> Vec<InputDevice> + Send>,
}

impl PillParts {
    pub fn system(prefs: PathBuf) -> Self {
        Self {
            prefs,
            play: Box::new(sounds::play),
            microphones: Box::new(|| CpalBackend::new().devices()),
        }
    }

    /// A rehearsal makes no sound and lists no device.
    fn silent(prefs: PathBuf) -> Self {
        Self {
            prefs,
            play: Box::new(|_| {}),
            microphones: Box::new(Vec::new),
        }
    }
}

/// The settings with the revision that makes compare and swap work, and the pill. The revision's
/// lock also keeps events in the order of the saves they report.
struct Board {
    store: SettingsStore,
    revision: Mutex<u64>,
    sink: Arc<dyn EventSink>,
    dictation: OnceLock<Dictation>,
    /// Held for moments only, and never while calling out.
    pill: Mutex<PillCore>,
    prefs_path: PathBuf,
    play: Box<dyn Fn(Sound) + Send + Sync>,
    catalog: Catalog,
    started: Instant,
    in_meeting: AtomicBool,
}

impl Board {
    fn lock_revision(&self) -> MutexGuard<'_, u64> {
        self.revision.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_pill(&self) -> MutexGuard<'_, PillCore> {
        self.pill.lock().unwrap_or_else(PoisonError::into_inner)
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
        written.map_err(disk_error)?;
        Ok(SaveOutcome::Saved { current })
    }

    /// A change from inside Rust, on the latest settings, so no save can undo it.
    fn change(&self, change: impl FnOnce(&mut Settings)) {
        let mut revision = self.lock_revision();
        let written = self.store.update(change);
        self.applied(&mut revision);
        if let Err(error) = written {
            self.toast(format!("Couldn't save settings: {error}"));
        }
    }

    /// Every save holds for this run, even one the disk refused.
    fn applied(&self, revision: &mut u64) -> Versioned {
        *revision += 1;
        let current = self.current(*revision);
        self.follow_settings(&current.settings);
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
            self.toast(format!("Added “{word}” to the dictionary"));
        }
    }

    fn follow_settings(&self, settings: &Settings) {
        let mut pill = self.lock_pill();
        pill.set_sounds(settings.media.sounds);
        pill.set_shown(settings.general.show_pill);
    }

    fn toast(&self, text: String) {
        self.lock_pill().show_toast(text.clone());
        self.sink.on_event(ShellEvent::Toast { text });
    }

    fn set_pill_state(&self, state: PillState) {
        let cue = self.lock_pill().set_state(state);
        if let Some(sound) = cue {
            (self.play)(sound);
        }
    }

    /// The pill hears of everything here, on the flow's thread, so what it shows never waits for
    /// Swift. The events still cross for whoever else wants them.
    fn deliver(&self, event: Event) {
        let event = match event {
            Event::Pill(state) => {
                self.set_pill_state(state.clone());
                ShellEvent::Pill { state }
            }
            Event::Level(value) => {
                self.lock_pill().set_level(value);
                ShellEvent::Level { value }
            }
            Event::Toast(text) => return self.toast(text),
            Event::Recent(transcripts) => {
                self.lock_pill().set_recent(transcripts.clone());
                ShellEvent::Recent { transcripts }
            }
            Event::Meeting(active) => {
                self.in_meeting.store(active, Ordering::SeqCst);
                self.lock_pill().hide_meeting_card();
                ShellEvent::Meeting { active }
            }
            Event::SuggestMeeting(app) => {
                self.lock_pill().suggest_meeting(app.clone());
                ShellEvent::SuggestMeeting { app }
            }
            Event::NoTextField(text) => {
                self.lock_pill().offer_copy(text.clone());
                ShellEvent::NoTextField { text }
            }
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
    keys: KeySlot,
}

impl Shell {
    /// Starts on `store`, with `start_flow` making the flow. Events reach the sink from then on.
    pub fn wired(
        store: SettingsStore,
        sink: Arc<dyn EventSink>,
        pill: PillParts,
        start_flow: impl FnOnce(Settings, flow::OnEvent) -> Dictation,
    ) -> Arc<Self> {
        let PillParts {
            prefs,
            play,
            microphones,
        } = pill;
        let board = Arc::new(Board {
            pill: Mutex::new(PillCore::new(Prefs::load(&prefs))),
            prefs_path: prefs,
            play,
            catalog: Catalog::bundled(),
            started: Instant::now(),
            in_meeting: AtomicBool::new(false),
            store,
            revision: Mutex::new(0),
            sink,
            dictation: OnceLock::new(),
        });
        board.follow_settings(&board.store.get());
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
        list_microphones(Arc::downgrade(&board), microphones);
        Arc::new(Self {
            board,
            dictation,
            keys: KeySlot::default(),
        })
    }
}

/// Fills the Microphone menu without holding anyone up on device enumeration.
fn list_microphones(
    board: Weak<Board>,
    microphones: Box<dyn FnOnce() -> Vec<InputDevice> + Send>,
) {
    let spawned = thread::Builder::new()
        .name("pill-microphones".into())
        .spawn(move || {
            let devices = microphones();
            if let Some(board) = board.upgrade() {
                board.lock_pill().set_microphones(devices);
            }
        });
    if let Err(err) = spawned {
        eprintln!("could not list microphones: {err}");
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
        let pill = PillParts::system(paths.pill.clone());
        Ok(Self::wired(store, sink, pill, |settings, on_event| {
            Dictation::start(flow::parts_at(&paths, settings, on_event))
        }))
    }

    /// A shell whose flow has nothing real under it (see [`rehearsal`]): it takes no microphone,
    /// loads no model, pastes nothing and makes no sound.
    #[uniffi::constructor]
    pub async fn rehearse(
        paths: Paths,
        sink: Arc<dyn EventSink>,
    ) -> Result<Arc<Self>, ShellError> {
        paths.check(&paths::real_folders())?;
        let store = SettingsStore::open(&paths.settings);
        let written = store.update(|settings| *settings = rehearsal::settings());
        written.map_err(disk_error)?;
        let pill = PillParts::silent(paths.pill.clone());
        Ok(Self::wired(store, sink, pill, |settings, on_event| {
            Dictation::start(rehearsal::parts(&paths, settings, on_event))
        }))
    }

    /// Puts things back before the app goes away: the sound, then any recording.
    pub async fn quit(&self) -> Result<(), ShellError> {
        self.dictation.close_media(MEDIA_CLOSE_LIMIT);
        self.dictation.close_meetings(MEETING_CLOSE_LIMIT);
        Ok(())
    }

    /// Starts listening to the keyboard, and keeps trying while the OS refuses, with the pill
    /// saying why. Only the app a person started may call this: it would race the real app.
    pub fn listen_to_keys(&self) -> Result<(), ShellError> {
        flow::listen_until_allowed(&self.dictation, &self.keys);
        Ok(())
    }

    /// One poll. `displays` are the screens, when the last frame wanted them.
    pub fn pill_tick(
        &self,
        cursor: Option<Point>,
        displays: Option<Vec<Display>>,
        escape: bool,
    ) -> Result<PillFrame, ShellError> {
        let now = self.board.started.elapsed();
        Ok(self.board.lock_pill().tick(now, cursor, displays, escape))
    }

    /// The left button went down on the capsule.
    pub fn pill_press(&self) -> Result<(), ShellError> {
        self.board.lock_pill().press();
        Ok(())
    }

    /// The left button came up. A drag to a new edge is saved, so this touches the disk.
    pub async fn pill_release(&self) -> Result<(), ShellError> {
        let (command, prefs) = {
            let mut pill = self.board.lock_pill();
            (pill.release(), pill.prefs().clone())
        };
        match command {
            Some(PillCommand::ToggleHandsFree) => self.dictation.on_pill(PillInput::Click),
            Some(PillCommand::EdgeChanged(_)) => save_prefs(&prefs, &self.board.prefs_path),
            None => {}
        }
        Ok(())
    }

    /// The card's answer, which means what the card asked.
    pub fn pill_answer(&self, accepted: bool) -> Result<(), ShellError> {
        let question = self.board.lock_pill().take_question();
        if accepted {
            match question {
                Some(Question::TakeNotes { .. }) => self.dictation.toggle_meeting(),
                Some(Question::CopyText(text)) => self.dictation.copy_text(text),
                None => {}
            }
        }
        Ok(())
    }

    /// Nothing, unless the pill is showing what the menu is for.
    pub fn pill_menu(&self) -> Result<Vec<MenuEntry>, ShellError> {
        let pill = self.board.lock_pill();
        Ok(if pill.menu_is_open_to() {
            menus::pill_menu(pill.recent(), pill.microphones())
        } else {
            Vec::new()
        })
    }

    pub fn status_menu(&self, debug: bool) -> Result<Vec<MenuEntry>, ShellError> {
        let in_meeting = self.board.in_meeting.load(Ordering::SeqCst);
        Ok(menus::status_menu(in_meeting, debug))
    }

    /// Does what the item picked says. Some items save settings, so this touches the disk.
    pub async fn menu_action(&self, action: MenuAction) -> Result<(), ShellError> {
        let board = &self.board;
        match action {
            MenuAction::PasteLast => self.dictation.paste_last(),
            MenuAction::PasteRecent { index } => self.dictation.paste_recent(index as usize),
            MenuAction::Microphone { id } => {
                board.change(|settings| rank_first(&mut settings.general.microphones, id));
            }
            MenuAction::Language { code } => {
                board.change(|settings| settings.general.languages = code.into_iter().collect());
            }
            MenuAction::PipelineQuality { tier } => {
                board.change(|settings| choose_tier(settings, tier, &board.catalog));
            }
            MenuAction::ToggleMeeting => self.dictation.toggle_meeting(),
            MenuAction::OpenMeetings => board.sink.on_event(ShellEvent::Open {
                window: AppWindow::Meetings,
            }),
            MenuAction::OpenSettings => board.sink.on_event(ShellEvent::Open {
                window: AppWindow::Settings,
            }),
            MenuAction::HideForAnHour => board.lock_pill().hide_for_an_hour(),
            MenuAction::Quit => board.sink.on_event(ShellEvent::QuitAsked),
            MenuAction::DebugState { state } => {
                board.lock_pill().drive_levels_by_hand();
                board.set_pill_state(state);
            }
            MenuAction::DebugSound { sound } => (board.play)(sound),
            MenuAction::DebugMicLevel => board.lock_pill().use_real_mic_level(),
        }
        Ok(())
    }

    /// Puts the pill in `scene`, the way the flow would. For `--pill-state` and `cargo xtask shots`.
    pub fn stage(&self, scene: PillScene) -> Result<(), ShellError> {
        let board = &self.board;
        match scene {
            PillScene::State { state } => {
                board.lock_pill().drive_levels_by_hand();
                board.deliver(Event::Pill(state));
            }
            PillScene::Toast { text } => board.deliver(Event::Toast(text)),
            PillScene::SuggestMeeting { app } => board.deliver(Event::SuggestMeeting(app)),
            PillScene::NoTextField { text } => board.deliver(Event::NoTextField(text)),
        }
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
