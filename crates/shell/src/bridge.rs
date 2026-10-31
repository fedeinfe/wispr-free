//! What Swift sees (spec, Bridge). One object, [`Shell`], with no window in it: it holds the flow
//! and the settings, takes every path from its caller and reports through one [`EventSink`].
//!
//! Every call returns a `Result`, because a panic in a call that can't throw is fatal in Swift.

use std::any::Any;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant};

use wf_audio::{AudioBackend as _, CpalBackend, InputDevice, sounds};
use wf_core::dictation::PillInput;
use wf_core::pill::dock::{Display, Point};
use wf_core::pill::{PillCommand, PillState, Sound};
use wf_core::settings::{ReplaceError, Replacement, Settings, SettingsStore, Snippet};
use wf_core::shortcut::Platform;
use wf_dictation::{Dictation, Event};
use wf_models::Catalog;
use wf_platform::desktop;
use wf_platform::keys::KeySignal;

use crate::app_windows::{OpenWindows, WindowOrder};
use crate::flow::{self, KeySlot, rank_first};
use crate::memory;
use crate::menus::{self, MenuAction, MenuEntry};
use crate::paths::{self, Paths};
use crate::pill::{PillCore, PillFrame, Question, save_prefs};
use crate::prefs::Prefs;
use crate::rehearsal;
use crate::settings::desk::{Desk, DeskParts};
use crate::settings::page::SettingsPage;
use crate::settings::state::PagesState;
use crate::settings::system::{self, SystemDesktop};
use crate::settings::values::{RuledChange, choose_tier};
use crate::settings::{self, Desktop, Mailbox, Pages};
use crate::windows::WINDOW_TEARDOWN;

pub use crate::app_windows::AppWindow;

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
    Pill {
        state: PillState,
    },
    Level {
        value: f32,
    },
    Toast {
        text: String,
    },
    Recent {
        transcripts: Vec<String>,
    },
    Meeting {
        active: bool,
    },
    SuggestMeeting {
        app: String,
    },
    NoTextField {
        text: String,
    },
    SettingsChanged {
        current: Versioned,
    },
    /// What the Settings pages show now. Sent when it changes, and before Settings opens.
    Pages {
        state: PagesState,
    },
    /// Ask where the models should go, starting at `start`, and say [`Shell::folder_picked`].
    PickFolder {
        start: PathBuf,
    },
    ShowInDock {
        visible: bool,
    },
    /// Make this window and show it. Say [`Shell::window_closed`] when it's gone.
    Open {
        window: AppWindow,
    },
    /// This window is open: bring it forward.
    Raise {
        window: AppWindow,
    },
    /// A menu asked the app to quit. The app calls [`Shell::quit`] on its way out.
    QuitAsked,
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

/// The error as the user reads it.
#[uniffi::export]
pub fn error_text(error: ShellError) -> String {
    error.to_string()
}

/// Runs on a thread of the flow's, so it must not wait: only hand the event on.
#[uniffi::export(with_foreign)]
pub trait EventSink: Send + Sync {
    fn on_event(&self, event: ShellEvent);
}

/// How the pages reach the machine, on a run that isn't the real app's. A run a person started gets
/// the keys and the microphone: any other could race the real app for them.
struct DevDesktop {
    system: SystemDesktop,
    sink: Arc<dyn EventSink>,
    by_hand: Arc<AtomicBool>,
    rehearsing: Arc<AtomicBool>,
}

impl DevDesktop {
    fn by_hand(&self) -> bool {
        self.by_hand.load(Ordering::SeqCst)
    }
}

impl Desktop for DevDesktop {
    /// Swift brings its windows forward.
    fn bring_to_front(&self) {}

    fn start_probes(&self, mailbox: &Mailbox) {
        if self.rehearsing.load(Ordering::SeqCst) {
            rehearsal::report_machine(mailbox);
        } else {
            system::start_probes(mailbox);
        }
    }

    /// The login entry has one name, and it's the installed app's: writing it here would start this
    /// bundle at login in that app's place, and removing it would stop that app. So nothing here
    /// touches it. **Ticket 15 gives this to `SystemDesktop`.**
    fn set_launch_at_login(&self, enabled: bool) -> io::Result<()> {
        if enabled {
            Err(io::Error::other("a dev run leaves it to the installed app"))
        } else {
            Ok(())
        }
    }

    fn set_dock_icon_visible(&self, visible: bool) {
        self.sink.on_event(ShellEvent::ShowInDock { visible });
    }

    fn open(&self, target: &str) -> io::Result<()> {
        if self.by_hand() {
            desktop::open(target)
        } else {
            eprintln!("would open {target}");
            Ok(())
        }
    }

    fn capture_keys(
        &self,
        on_signal: Box<dyn FnMut(KeySignal) + Send>,
    ) -> Result<Box<dyn Any>, String> {
        if self.by_hand() {
            self.system.capture_keys(on_signal)
        } else {
            Err("Only a run started by hand records keys.".to_owned())
        }
    }

    fn test_microphone(
        &self,
        ranking: Vec<String>,
        on_level: Box<dyn FnMut(f32) + Send>,
    ) -> Result<Box<dyn Any>, String> {
        if self.by_hand() {
            self.system.test_microphone(ranking, on_level)
        } else {
            Err("Only a run started by hand opens the microphone.".to_owned())
        }
    }

    fn pick_folder(&self, start: &Path, _done: Box<dyn FnOnce(PathBuf)>) {
        self.sink.on_event(ShellEvent::PickFolder {
            start: start.to_owned(),
        });
    }

    fn before_delete_all(&self) {
        self.system.before_delete_all();
    }

    fn look_for_microphones(&self, mailbox: &Mailbox) {
        self.system.look_for_microphones(mailbox);
    }
}

/// Makes the pages' way to the machine, on their own thread.
pub type MakeDesktop = Box<dyn FnOnce() -> Rc<dyn Desktop> + Send>;

/// What the pages are made of, kept until something first needs them.
struct PagesRecipe {
    paths: Paths,
    make_desktop: MakeDesktop,
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

/// The settings, the pill and the Settings pages. The store's watcher keeps events in the order of
/// the saves they report.
struct Board {
    store: Arc<SettingsStore>,
    sink: Arc<dyn EventSink>,
    dictation: OnceLock<Dictation>,
    /// Held for moments only, and never while calling out.
    pill: Mutex<PillCore>,
    /// Held for moments only, and never while calling out.
    windows: Mutex<OpenWindows>,
    prefs_path: PathBuf,
    play: Box<dyn Fn(Sound) + Send + Sync>,
    catalog: Arc<Catalog>,
    started: Instant,
    in_meeting: AtomicBool,
    /// A person started this run.
    by_hand: Arc<AtomicBool>,
    /// Nothing real is under this run, not even the machine the pages describe.
    rehearsing: Arc<AtomicBool>,
    /// Made when something first needs the pages, so an app that never opens Settings pays nothing.
    pages: OnceLock<Option<Desk>>,
    recipe: Mutex<Option<PagesRecipe>>,
}

impl Board {
    fn lock_pill(&self) -> MutexGuard<'_, PillCore> {
        self.pill.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_windows(&self) -> MutexGuard<'_, OpenWindows> {
        self.windows.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The thread with the Settings pages, started on first need.
    fn desk(self: &Arc<Self>) -> Option<&Desk> {
        self.pages.get_or_init(|| self.start_desk()).as_ref()
    }

    fn start_desk(self: &Arc<Self>) -> Option<Desk> {
        let recipe = self
            .recipe
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()?;
        let sink = Arc::clone(&self.sink);
        let levels = Arc::clone(&self.sink);
        let parts = DeskParts {
            store: Arc::clone(&self.store),
            catalog: Arc::clone(&self.catalog),
            paths: settings::Paths {
                data_dir: recipe.paths.data.clone(),
                export_dir: recipe.paths.exports.clone(),
                model_fence: recipe.paths.model_fence(),
            },
            platform: Platform::current(),
            page: self.lock_pill().prefs().settings_page,
            make_desktop: recipe.make_desktop,
            publish: Box::new(move |state| sink.on_event(ShellEvent::Pages { state })),
            level: Box::new(move |value| levels.on_event(ShellEvent::Level { value })),
        };
        Desk::start(parts)
            .inspect_err(|error| eprintln!("could not start the settings pages: {error}"))
            .ok()
    }

    /// Runs `work` on the pages, after what was asked of them before.
    fn pages_do(self: &Arc<Self>, work: impl FnOnce(&Pages) + Send + 'static) {
        if let Some(desk) = self.desk() {
            desk.work(work);
        }
    }

    /// Asks for `window`, so the rule decides what the app does about it.
    fn ask_for(self: &Arc<Self>, window: AppWindow) {
        let order = self.lock_windows().ask(window);
        if order == WindowOrder::Open(AppWindow::Settings) {
            self.fill_pages();
        }
        self.sink.on_event(match order {
            WindowOrder::Open(window) => ShellEvent::Open { window },
            WindowOrder::Raise(window) => ShellEvent::Raise { window },
        });
    }

    /// Looks again at what's slow to learn. Swift has the pages by the time this returns, so the
    /// window doesn't open empty.
    fn fill_pages(self: &Arc<Self>) {
        self.pages_do(|pages| {
            pages.start_probes();
            pages.recheck_models_folder();
        });
        if let Some(desk) = self.desk() {
            desk.ask();
        }
    }

    fn current(&self) -> Versioned {
        let (settings, revision) = self.store.revised();
        Versioned { settings, revision }
    }

    fn save(&self, settings: Settings, based_on: u64) -> Result<SaveOutcome, ShellError> {
        match self.store.replace(settings, based_on) {
            Ok(()) => Ok(SaveOutcome::Saved {
                current: self.current(),
            }),
            Err(ReplaceError::Stale) => Ok(SaveOutcome::Stale {
                current: self.current(),
            }),
            Err(ReplaceError::Disk(error)) => Err(disk_error(error)),
        }
    }

    /// A change from inside Rust, on the latest settings, so no save can undo it.
    fn change(&self, change: impl FnOnce(&mut Settings)) {
        if let Err(error) = self.store.update(change) {
            self.toast(format!("Couldn't save settings: {error}"));
        }
    }

    /// Every save lands here, from anywhere, under the store's lock. It holds for this run even if
    /// the disk refused it.
    fn applied(&self, settings: &Settings, revision: u64) {
        self.follow_settings(settings);
        if let Some(dictation) = self.dictation.get() {
            dictation.settings_changed(settings.clone());
        }
        self.sink.on_event(ShellEvent::SettingsChanged {
            current: Versioned {
                settings: settings.clone(),
                revision,
            },
        });
        if let Some(Some(desk)) = self.pages.get() {
            desk.follow();
        }
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
    /// Starts on `store`, with `start_flow` making the flow and `paths` for the Settings pages,
    /// which are made when first needed. Events reach the sink from then on.
    pub fn wired(
        store: SettingsStore,
        sink: Arc<dyn EventSink>,
        pill: PillParts,
        paths: Paths,
        start_flow: impl FnOnce(Settings, flow::OnEvent) -> Dictation,
    ) -> Arc<Self> {
        Self::wired_with(store, sink, pill, paths, None, start_flow)
    }

    /// [`Shell::wired`] with the pages on `make_desktop`, for a test.
    pub fn wired_with(
        store: SettingsStore,
        sink: Arc<dyn EventSink>,
        pill: PillParts,
        paths: Paths,
        make_desktop: Option<MakeDesktop>,
        start_flow: impl FnOnce(Settings, flow::OnEvent) -> Dictation,
    ) -> Arc<Self> {
        let PillParts {
            prefs,
            play,
            microphones,
        } = pill;
        let board = Arc::new(Board {
            pill: Mutex::new(PillCore::new(Prefs::load(&prefs))),
            windows: Mutex::default(),
            prefs_path: prefs,
            play,
            catalog: Arc::new(Catalog::bundled()),
            started: Instant::now(),
            in_meeting: AtomicBool::new(false),
            by_hand: Arc::default(),
            rehearsing: Arc::default(),
            pages: OnceLock::new(),
            recipe: Mutex::default(),
            store: Arc::new(store),
            sink,
            dictation: OnceLock::new(),
        });
        board.follow_settings(&board.store.get());
        // Weak, or the flow and the store would keep the board alive and the board them.
        let watching = Arc::downgrade(&board);
        board.store.watch(Box::new(move |settings, revision| {
            if let Some(board) = Weak::upgrade(&watching) {
                board.applied(settings, revision);
            }
        }));
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
        let make_desktop = make_desktop.unwrap_or_else(|| dev_desktop(&board, &dictation));
        *board.recipe.lock().unwrap_or_else(PoisonError::into_inner) = Some(PagesRecipe {
            paths,
            make_desktop,
        });
        list_microphones(Arc::downgrade(&board), microphones);
        Arc::new(Self {
            board,
            dictation,
            keys: KeySlot::default(),
        })
    }
}

fn dev_desktop(board: &Board, dictation: &Dictation) -> MakeDesktop {
    let (sink, by_hand) = (Arc::clone(&board.sink), Arc::clone(&board.by_hand));
    let rehearsing = Arc::clone(&board.rehearsing);
    let dictation = dictation.clone();
    Box::new(move || {
        Rc::new(DevDesktop {
            system: SystemDesktop {
                dictation,
                // Swift's picker runs on its own and comes back through `folder_picked`.
                pick_folder: Rc::new(|_, _| {}),
            },
            sink,
            by_hand,
            rehearsing,
        })
    })
}

/// Returns what a closed window freed once the OS window is gone too.
fn release_memory_after_teardown() {
    let spawned = thread::Builder::new()
        .name("window-teardown".into())
        .spawn(|| {
            thread::sleep(WINDOW_TEARDOWN);
            memory::release_freed();
        });
    if let Err(err) = spawned {
        eprintln!("could not release a closed window's memory: {err}");
    }
}

/// Fills the Microphone menu without holding anyone up on device enumeration.
fn list_microphones(board: Weak<Board>, microphones: Box<dyn FnOnce() -> Vec<InputDevice> + Send>) {
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
        let for_pages = paths.clone();
        Ok(Self::wired(
            store,
            sink,
            pill,
            for_pages,
            |settings, on_event| Dictation::start(flow::parts_at(&paths, settings, on_event)),
        ))
    }

    /// A shell whose flow has nothing real under it (see [`rehearsal`]): it takes no microphone,
    /// loads no model, pastes nothing and makes no sound.
    #[uniffi::constructor]
    pub async fn rehearse(paths: Paths, sink: Arc<dyn EventSink>) -> Result<Arc<Self>, ShellError> {
        paths.check(&paths::real_folders())?;
        let store = SettingsStore::open(&paths.settings);
        let written = store.update(|settings| *settings = rehearsal::settings());
        written.map_err(disk_error)?;
        let pill = PillParts::silent(paths.pill.clone());
        let for_pages = paths.clone();
        let shell = Self::wired(store, sink, pill, for_pages, |settings, on_event| {
            Dictation::start(rehearsal::parts(&paths, settings, on_event))
        });
        shell.board.rehearsing.store(true, Ordering::SeqCst);
        Ok(shell)
    }

    /// Puts things back before the app goes away: the sound, then any recording.
    pub async fn quit(&self) -> Result<(), ShellError> {
        if let Some(Some(desk)) = self.board.pages.get() {
            desk.stop();
        }
        self.dictation.close_media(MEDIA_CLOSE_LIMIT);
        self.dictation.close_meetings(MEETING_CLOSE_LIMIT);
        Ok(())
    }

    /// Starts listening to the keyboard, and keeps trying while the OS refuses, with the pill
    /// saying why. Only the app a person started may call this: it would race the real app.
    pub fn listen_to_keys(&self) -> Result<(), ShellError> {
        self.board.by_hand.store(true, Ordering::SeqCst);
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
        let setup_needed = !self.board.store.get().onboarding.finished;
        Ok(menus::status_menu(in_meeting, setup_needed, debug))
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
            MenuAction::OpenMeetings => board.ask_for(AppWindow::Meetings),
            MenuAction::OpenOnboarding => board.ask_for(AppWindow::Onboarding),
            MenuAction::OpenSettings => board.ask_for(AppWindow::Settings),
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

    /// `window` is closed and Swift has let go of it.
    pub fn window_closed(&self, window: AppWindow) -> Result<(), ShellError> {
        self.board.lock_windows().closed(window);
        if window == AppWindow::Settings {
            self.board.pages_do(Pages::window_closed);
        }
        release_memory_after_teardown();
        Ok(())
    }

    /// The page Settings opens on, which is the one it was last on.
    pub fn settings_page(&self) -> Result<SettingsPage, ShellError> {
        Ok(self.board.lock_pill().prefs().settings_page)
    }

    /// The user moved to `page`. It's saved, so this touches the disk.
    pub async fn set_settings_page(&self, page: SettingsPage) -> Result<(), ShellError> {
        let prefs = {
            let mut pill = self.board.lock_pill();
            pill.set_settings_page(page);
            pill.prefs().clone()
        };
        save_prefs(&prefs, &self.board.prefs_path);
        if let Some(desk) = self.board.desk() {
            desk.set_page(page);
        }
        Ok(())
    }

    /// One setting with a rule or a side effect. The rest go through [`Shell::save`].
    pub fn change_setting(&self, change: RuledChange) -> Result<(), ShellError> {
        self.board.pages_do(move |pages| pages.change(change));
        Ok(())
    }

    /// Takes the next keys as this shortcut, in `Action::ALL` order.
    pub fn record_shortcut(&self, index: i32) -> Result<(), ShellError> {
        self.board
            .pages_do(move |pages| pages.record_shortcut(index));
        Ok(())
    }

    pub fn stop_recording_shortcut(&self) -> Result<(), ShellError> {
        self.board.pages_do(Pages::stop_recording);
        Ok(())
    }

    pub fn reset_shortcut(&self, index: i32) -> Result<(), ShellError> {
        self.board
            .pages_do(move |pages| pages.reset_shortcut(index));
        Ok(())
    }

    pub fn set_microphone_auto(&self, on: bool) -> Result<(), ShellError> {
        self.board
            .pages_do(move |pages| pages.set_microphone_auto(on));
        Ok(())
    }

    pub fn move_microphone(&self, index: i32, by: i32) -> Result<(), ShellError> {
        self.board
            .pages_do(move |pages| pages.move_microphone(index, by));
        Ok(())
    }

    pub fn toggle_microphone_test(&self) -> Result<(), ShellError> {
        self.board.pages_do(Pages::toggle_microphone_test);
        Ok(())
    }

    pub fn set_languages_auto(&self, on: bool) -> Result<(), ShellError> {
        self.board
            .pages_do(move |pages| pages.set_languages_auto(on));
        Ok(())
    }

    pub fn toggle_language(&self, index: i32) -> Result<(), ShellError> {
        self.board
            .pages_do(move |pages| pages.toggle_language(index));
        Ok(())
    }

    pub fn add_word(&self, word: String) -> Result<(), ShellError> {
        self.board.pages_do(move |pages| pages.add_word(&word));
        Ok(())
    }

    pub fn remove_word(&self, index: i32) -> Result<(), ShellError> {
        self.board.pages_do(move |pages| pages.remove_word(index));
        Ok(())
    }

    pub fn add_replacement(&self) -> Result<(), ShellError> {
        self.board.pages_do(Pages::add_replacement);
        Ok(())
    }

    pub fn edit_replacement(&self, index: u32, row: Replacement) -> Result<(), ShellError> {
        self.board
            .pages_do(move |pages| pages.edit_replacement(index as usize, row));
        Ok(())
    }

    pub fn remove_replacement(&self, index: u32) -> Result<(), ShellError> {
        self.board
            .pages_do(move |pages| pages.remove_replacement(index as usize));
        Ok(())
    }

    pub fn add_snippet(&self) -> Result<(), ShellError> {
        self.board.pages_do(Pages::add_snippet);
        Ok(())
    }

    pub fn edit_snippet(&self, index: u32, row: Snippet) -> Result<(), ShellError> {
        self.board
            .pages_do(move |pages| pages.edit_snippet(index as usize, row));
        Ok(())
    }

    pub fn remove_snippet(&self, index: u32) -> Result<(), ShellError> {
        self.board
            .pages_do(move |pages| pages.remove_snippet(index as usize));
        Ok(())
    }

    /// `index` is the model's place in the catalog. It goes on after its window closes.
    pub fn download(&self, index: i32) -> Result<(), ShellError> {
        self.board
            .pages_do(move |pages| pages.download_model(index));
        Ok(())
    }

    pub fn cancel_download(&self) -> Result<(), ShellError> {
        self.board.pages_do(Pages::cancel_download);
        Ok(())
    }

    pub fn verify(&self, index: i32) -> Result<(), ShellError> {
        self.board.pages_do(move |pages| pages.verify_model(index));
        Ok(())
    }

    pub fn delete_model(&self, index: i32) -> Result<(), ShellError> {
        self.board.pages_do(move |pages| pages.delete_model(index));
        Ok(())
    }

    /// Asks for a folder, unless a job rules it out. It comes back as [`ShellEvent::PickFolder`].
    pub fn choose_models_folder(&self) -> Result<(), ShellError> {
        self.board.pages_do(Pages::choose_models_folder);
        Ok(())
    }

    pub fn folder_picked(&self, folder: PathBuf) -> Result<(), ShellError> {
        self.board
            .pages_do(move |pages| pages.folder_picked(folder));
        Ok(())
    }

    pub fn use_default_models_folder(&self) -> Result<(), ShellError> {
        self.board.pages_do(Pages::use_default_models_folder);
        Ok(())
    }

    pub fn open_models_folder(&self) -> Result<(), ShellError> {
        self.board.pages_do(Pages::open_models_folder);
        Ok(())
    }

    pub fn recheck_models_folder(&self) -> Result<(), ShellError> {
        self.board.pages_do(Pages::recheck_models_folder);
        Ok(())
    }

    /// Moves the models to the folder waiting for an answer. It goes on after its window closes.
    pub fn move_folder(&self) -> Result<(), ShellError> {
        self.board.pages_do(Pages::move_to_pending_folder);
        Ok(())
    }

    pub fn switch_folder(&self) -> Result<(), ShellError> {
        self.board.pages_do(Pages::switch_to_pending_folder);
        Ok(())
    }

    pub fn cancel_pending_folder(&self) -> Result<(), ShellError> {
        self.board.pages_do(Pages::cancel_pending_folder);
        Ok(())
    }

    pub fn cancel_move(&self) -> Result<(), ShellError> {
        self.board.pages_do(Pages::cancel_move);
        Ok(())
    }

    pub fn open_data_folder(&self) -> Result<(), ShellError> {
        self.board.pages_do(Pages::open_data_folder);
        Ok(())
    }

    pub fn export_all(&self) -> Result<(), ShellError> {
        self.board.pages_do(Pages::export_all);
        Ok(())
    }

    pub fn delete_all(&self) -> Result<(), ShellError> {
        self.board.pages_do(Pages::delete_all);
        Ok(())
    }

    pub fn forget_history(&self) -> Result<(), ShellError> {
        self.dictation.forget_history();
        Ok(())
    }

    pub fn check_for_updates(&self) -> Result<(), ShellError> {
        self.board.pages_do(Pages::check_for_updates);
        Ok(())
    }

    /// What the pages show, once they've done all they were told, for a test.
    pub async fn pages_state(&self) -> Result<Option<PagesState>, ShellError> {
        Ok(self.board.desk().and_then(Desk::ask))
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
        Ok(self.board.current())
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
