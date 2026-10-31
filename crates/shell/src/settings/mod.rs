//! The Settings pages (spec §5), with no window. [`Pages`] ties them to the settings store and the
//! rest of the app, and says what to show through a [`View`]. Onboarding gets the Models machinery
//! from here without a second copy.

pub mod data;
pub mod desk;
pub mod page;
pub mod rows;
pub mod state;
pub mod system;
pub mod values;

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use wf_audio::InputDevice;
use wf_core::settings::{PipelineTier, Replacement, Settings, SettingsStore, Snippet};
use wf_core::shortcut::recorder::{RecorderStep, ShortcutRecorder, reset_to_default};
use wf_core::shortcut::{Action, Platform};
use wf_dictation::plan::vad_forced;
use wf_models::{Catalog, DownloadError, ModelEntry, ModelStatus, ModelStore};
use wf_platform::keys::KeySignal;

use crate::clock::{Clock, Timer};
use crate::lists::Rows;
use rows::{
    LANGUAGES, LanguageRow, MicrophoneLine, ModelRow, ShortcutRow, downloaded_summary,
    format_bytes, language_rows, microphone_lines, model_row, model_row_unavailable, shortcut_rows,
};
use values::{ModelChoices, RuledChange, SettingsValues, apply_values, models_in_use, values_of};

const RELEASES_URL: &str = "https://github.com/fedeinfe/wispr-free/releases";

/// A recording swallows every key on the system, so one left open must not eat the user's typing
/// in another app. It ends after this long without a key.
pub const RECORDING_IDLE_LIMIT: Duration = Duration::from_secs(15);

/// What the window needs from the OS, behind a seam so tests touch nothing real.
pub trait Desktop {
    /// Gives an opened window the foreground, which an app without a Dock icon doesn't get.
    fn bring_to_front(&self);
    /// Starts finding out what is slow to learn, reported through `mailbox`.
    fn start_probes(&self, mailbox: &Mailbox);
    fn set_launch_at_login(&self, enabled: bool) -> io::Result<()>;
    fn set_dock_icon_visible(&self, visible: bool);
    fn open(&self, target: &str) -> io::Result<()>;
    /// Captures keys globally, swallowing them, until the returned guard is dropped. `on_signal`
    /// runs on another thread.
    fn capture_keys(
        &self,
        on_signal: Box<dyn FnMut(KeySignal) + Send>,
    ) -> Result<Box<dyn Any>, String>;
    /// Listens to the best-ranked mic until the returned guard is dropped. `on_level` (0..1) runs
    /// on another thread.
    fn test_microphone(
        &self,
        ranking: Vec<String>,
        on_level: Box<dyn FnMut(f32) + Send>,
    ) -> Result<Box<dyn Any>, String>;
    /// Asks for a folder, starting at `start`. `done` runs on the UI thread, only if one is picked.
    fn pick_folder(&self, start: &Path, done: Box<dyn FnOnce(PathBuf)>);
    /// Lets go of what lives in the data folder, which is about to be removed.
    fn before_delete_all(&self) {}
    /// Reports the microphones again, as [`Event::Microphones`] through `mailbox`.
    fn look_for_microphones(&self, _mailbox: &Mailbox) {}
}

#[derive(Debug, Clone)]
pub struct Paths {
    /// History, meetings, and models unless the user chose another folder for them. Delete all
    /// removes this folder.
    pub data_dir: PathBuf,
    /// Where Export all puts its folder.
    pub export_dir: PathBuf,
    /// The one folder models may change in, wherever they're read from. `None` is no limit.
    pub model_fence: Option<PathBuf>,
}

/// Everything a window is built from.
#[derive(Clone)]
pub struct SettingsParts {
    pub store: Arc<SettingsStore>,
    pub catalog: Arc<Catalog>,
    pub desktop: Rc<dyn Desktop>,
    /// Told about every change the rest of the running app may care about.
    pub on_change: OnChange,
    pub paths: Paths,
    pub platform: Platform,
}

pub type OnChange = Rc<dyn Fn(&Settings)>;

/// What other threads report to the window.
#[derive(Debug)]
pub enum Event {
    Key(KeySignal),
    Level(f32),
    Microphones {
        devices: Vec<InputDevice>,
        default: Option<String>,
    },
    Hardware {
        summary: String,
        recommended: PipelineTier,
    },
    ModelProgress {
        index: usize,
        fraction: f32,
    },
    ModelDone {
        index: usize,
        outcome: Result<Option<String>, String>,
    },
    MoveProgress(f32),
    MoveDone(Result<(), String>),
}

/// Delivers [`Event`]s to the window from any thread. Dropped silently once the window is gone.
#[derive(Clone)]
pub struct Mailbox {
    events: Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Mailbox {
    pub fn post(&self, event: Event) {
        if self.events.send(event).is_ok() {
            (self.wake)();
        }
    }
}

/// What the pages know before anything is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageFacts {
    pub is_macos: bool,
    pub version: String,
    pub data_folder: String,
    /// `asr_rows[i]` is the row in the list of models of the `i`th ASR option, and so on.
    pub asr_rows: Vec<i32>,
    pub vad_rows: Vec<i32>,
    pub llm_rows: Vec<i32>,
}

/// One thing for the pages to show now.
#[derive(Debug, Clone, PartialEq)]
pub enum Shown {
    Facts(PageFacts),
    Notice(String),
    Values(Box<SettingsValues>),
    /// The ASR engine needs VAD, so the setting doesn't apply.
    VadForced(bool),
    Shortcuts(Vec<ShortcutRow>),
    /// Which row of the shortcuts is recording, or -1.
    RecordingAction(i32),
    RecordingKeys(String),
    ShortcutError(String),
    Microphones(Vec<MicrophoneLine>),
    MicrophoneAuto(bool),
    MicrophoneTesting(bool),
    MicrophoneLevel(f32),
    MicrophoneError(String),
    LanguagesAuto(bool),
    Languages(Vec<LanguageRow>),
    Dictionary(Vec<String>),
    Replacements(Rows<Replacement>),
    Snippets(Rows<Snippet>),
    HardwareSummary(String),
    RecommendedTier(PipelineTier),
    Models(Rows<ModelRow>),
    ModelsInUseSummary(String),
    DownloadedSummary(String),
    /// What the pipeline's three model menus offer.
    ModelMenus {
        asr: Vec<String>,
        vad: Vec<String>,
        llm: Vec<String>,
    },
    ModelBusy(bool),
    ModelError(String),
    ModelsFolder(String),
    ModelsFolderCustom(bool),
    ModelsFolderMissing(bool),
    /// Models here can be read but not added, moved or deleted.
    ModelsReadOnly(bool),
    PendingFolder(String),
    PendingSummary(String),
    MovingModels(bool),
    MoveProgress(f32),
    DataStatus(String),
}

/// Where the pages say what to show. Takes no calls back in.
pub type View = Rc<dyn Fn(Shown)>;

struct Recording {
    action: Action,
    recorder: ShortcutRecorder,
    _capture: Box<dyn Any>,
}

/// One open set of pages. Dropping it closes everything they started.
pub struct Pages {
    me: Weak<Pages>,
    parts: SettingsParts,
    view: View,
    choices: ModelChoices,
    mailbox: Mailbox,
    events: Receiver<Event>,
    recording: RefCell<Option<Recording>>,
    recording_timeout: Box<dyn Timer>,
    microphone_test: RefCell<Option<Box<dyn Any>>>,
    devices: RefCell<(Vec<InputDevice>, Option<String>)>,
    devices_known: Cell<bool>,
    on_microphones: RefCell<Option<Box<dyn Fn()>>>,
    models: RefCell<Vec<ModelRow>>,
    model_task: Cell<Option<usize>>,
    /// Downloads waiting for the one running.
    model_queue: RefCell<VecDeque<usize>>,
    cancel_download: Arc<AtomicBool>,
    model_store: RefCell<Arc<ModelStore>>,
    models_folder_missing: Cell<bool>,
    /// The folder waiting for the user to say what happens to their models. The inner `None` is
    /// the app's default folder.
    pending_folder: RefCell<Option<Option<PathBuf>>>,
    moving_models: Cell<bool>,
    recommended_tier: Cell<Option<PipelineTier>>,
    replacements: RefCell<Vec<Replacement>>,
    snippets: RefCell<Vec<Snippet>>,
}

impl Pages {
    /// `wake` runs on any thread when events arrive, and must end in [`Pages::drain_events`] on the
    /// UI thread.
    pub fn new(
        parts: SettingsParts,
        view: View,
        clock: &dyn Clock,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Rc<Self> {
        let (sender, events) = mpsc::channel();
        let mailbox = Mailbox {
            events: sender,
            wake,
        };
        let choices = ModelChoices::from_catalog(&parts.catalog);
        let model_store = RefCell::new(Arc::new(configured_store(&parts)));
        let recording_timeout = clock.timer();
        let pages = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            parts,
            view,
            choices,
            mailbox,
            events,
            recording: RefCell::default(),
            recording_timeout,
            microphone_test: RefCell::default(),
            devices: RefCell::default(),
            devices_known: Cell::default(),
            on_microphones: RefCell::default(),
            models: RefCell::default(),
            model_task: Cell::default(),
            model_queue: RefCell::default(),
            cancel_download: Arc::default(),
            model_store,
            models_folder_missing: Cell::default(),
            pending_folder: RefCell::default(),
            moving_models: Cell::default(),
            recommended_tier: Cell::default(),
            replacements: RefCell::default(),
            snippets: RefCell::default(),
        });
        pages.show_facts();
        pages.show_everything();
        pages
    }

    fn show(&self, shown: Shown) {
        (self.view)(shown);
    }

    fn show_facts(&self) {
        self.show(Shown::Facts(PageFacts {
            is_macos: self.parts.platform == Platform::MacOs,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            data_folder: self.parts.paths.data_dir.to_string_lossy().into_owned(),
            asr_rows: self.catalog_rows(&self.choices.asr_ids()),
            vad_rows: self.catalog_rows(&self.choices.vad_ids()),
            llm_rows: self.catalog_rows(&self.choices.llm_ids()),
        }));
    }

    pub fn mailbox(&self) -> Mailbox {
        self.mailbox.clone()
    }

    pub fn start_probes(&self) {
        self.parts.desktop.start_probes(&self.mailbox);
    }

    pub fn bring_to_front(&self) {
        self.parts.desktop.bring_to_front();
    }

    /// Re-reads what is on disk for the Models page.
    pub fn refresh_models(&self) {
        self.show_models();
    }

    /// Shows the stored settings again.
    pub fn refresh(&self) {
        self.show_everything();
    }

    /// Something else changed the settings. Shows what they decide, and leaves alone what's being
    /// typed, recorded or run on the pages.
    pub fn follow(&self) {
        let settings = self.settings();
        self.show_values(&settings);
        self.show_shortcuts(&settings);
        self.show_microphones();
        self.show(Shown::LanguagesAuto(settings.general.languages.is_empty()));
        self.show_languages(&settings);
        self.show_dictionary(&settings);
        self.show_models_in_use();
    }

    /// The window that showed the pages is gone, so nothing may keep the keys or the microphone.
    /// Downloads and moves carry on: they belong to the run.
    pub fn window_closed(&self) {
        if self.recording.borrow().is_some() {
            self.stop_recording();
        }
        if self.microphone_test.borrow().is_some() {
            self.toggle_microphone_test();
        }
    }

    /// The connected microphones, once a look has found out.
    pub fn microphones(&self) -> Option<Vec<InputDevice>> {
        let devices = self.devices.borrow();
        self.devices_known.get().then(|| devices.0.clone())
    }

    /// Calls `seen` on the UI thread after each look at the microphones. It must hold only weak
    /// references to whatever owns these pages.
    pub fn on_microphones(&self, seen: impl Fn() + 'static) {
        *self.on_microphones.borrow_mut() = Some(Box::new(seen));
    }

    /// Looks for microphones again, for whoever waits for one to be plugged in.
    pub fn look_for_microphones(&self) {
        self.parts.desktop.look_for_microphones(&self.mailbox);
    }

    /// Whether the models folder was gone at the last look.
    pub fn models_folder_missing(&self) -> bool {
        self.models_folder_missing.get()
    }

    /// The tier the hardware suggests, once the probe has reported.
    pub fn recommended_tier(&self) -> Option<PipelineTier> {
        self.recommended_tier.get()
    }

    /// Puts `text` at the top of the pages.
    pub fn notice(&self, text: String) {
        self.show(Shown::Notice(text));
    }

    fn settings(&self) -> Settings {
        self.parts.store.get()
    }

    /// Changes the settings and reports a failed save at the top of the window.
    fn update(&self, change: impl FnOnce(&mut Settings)) {
        if let Err(error) = self.parts.store.update(change) {
            self.notice(format!("Couldn't save settings: {error}"));
        }
    }

    fn show_everything(&self) {
        let settings = self.settings();
        self.show_values(&settings);
        self.show_shortcuts(&settings);
        self.show_microphones();
        self.show(Shown::LanguagesAuto(settings.general.languages.is_empty()));
        self.show_languages(&settings);
        self.show_dictionary(&settings);
        let replacements = settings.style.replacements.clone();
        self.change_replacements(Rows::All(replacements));
        self.change_snippets(Rows::All(settings.style.snippets.clone()));
        self.show_models();
        if let Some(path) = self.parts.store.set_aside() {
            self.notice(format!(
                "The settings file was damaged, so defaults are in use. A copy is at {}.",
                path.display()
            ));
        }
    }

    fn show_values(&self, settings: &Settings) {
        let values = values_of(settings, &self.parts.catalog, &self.choices);
        self.show(Shown::Values(Box::new(values)));
        let forced = vad_forced(settings, &self.parts.catalog);
        self.show(Shown::VadForced(forced));
    }

    fn show_shortcuts(&self, settings: &Settings) {
        let rows = shortcut_rows(settings, self.parts.platform);
        self.show(Shown::Shortcuts(rows));
    }

    fn show_languages(&self, settings: &Settings) {
        self.show(Shown::Languages(language_rows(&settings.general.languages)));
    }

    fn show_dictionary(&self, settings: &Settings) {
        self.show(Shown::Dictionary(settings.style.dictionary.clone()));
    }

    /// Saves what the pages show, the whole of it. Every control that edits a value ends here.
    pub fn commit(&self, values: &SettingsValues) {
        self.notice(String::new());
        let before = self.settings();
        self.update(|settings| {
            apply_values(settings, values, &self.parts.catalog, &self.choices);
        });
        let after = self.settings();

        let wanted_login = after.general.launch_at_login;
        if wanted_login != before.general.launch_at_login
            && let Err(error) = self.parts.desktop.set_launch_at_login(wanted_login)
        {
            self.notice(format!("Couldn't change launch at login: {error}"));
            self.update(|settings| settings.general.launch_at_login = !wanted_login);
        }
        if after.general.show_in_dock != before.general.show_in_dock {
            self.parts
                .desktop
                .set_dock_icon_visible(after.general.show_in_dock);
        }
        let settings = self.settings();
        self.show_values(&settings);
        self.show_models_in_use();
        (self.parts.on_change)(&settings);
    }

    /// One setting with a rule or a side effect, on the latest values.
    pub fn change(&self, change: RuledChange) {
        let mut values = values_of(&self.settings(), &self.parts.catalog, &self.choices);
        change.apply(&mut values);
        self.commit(&values);
    }

    /// Runs what other threads have reported so far.
    pub fn drain_events(&self) {
        while let Ok(event) = self.events.try_recv() {
            self.handle(event);
        }
    }

    pub fn handle(&self, event: Event) {
        match event {
            Event::Key(signal) => self.on_key(signal),
            Event::Level(level) => {
                if self.microphone_test.borrow().is_some() {
                    self.show(Shown::MicrophoneLevel(level));
                }
            }
            Event::Microphones { devices, default } => {
                *self.devices.borrow_mut() = (devices, default);
                self.devices_known.set(true);
                self.show_microphones();
                if let Some(seen) = &*self.on_microphones.borrow() {
                    seen();
                }
            }
            Event::Hardware {
                summary,
                recommended,
            } => {
                self.show(Shown::HardwareSummary(summary));
                self.recommended_tier.set(Some(recommended));
                self.show(Shown::RecommendedTier(recommended));
            }
            Event::ModelProgress { index, fraction } => {
                let row = self.models.borrow().get(index).cloned();
                if let Some(mut row) = row {
                    row.progress = fraction;
                    row.status = format!("Downloading… {:.0}%", fraction * 100.0);
                    self.change_models(Rows::One(index, row));
                }
            }
            Event::ModelDone { index, outcome } => self.finish_model_task(index, outcome),
            Event::MoveProgress(fraction) => self.show(Shown::MoveProgress(fraction)),
            Event::MoveDone(outcome) => self.finish_move(outcome),
        }
    }

    /// Starts over the time a recording is left open.
    fn keep_recording_open(&self) {
        let weak = self.me.clone();
        self.recording_timeout.once(
            RECORDING_IDLE_LIMIT,
            Box::new(move || {
                if let Some(this) = weak.upgrade() {
                    this.stop_recording();
                }
            }),
        );
    }

    pub fn record_shortcut(&self, index: i32) {
        let Some(&action) = Action::ALL.get(index as usize) else {
            return;
        };
        let mailbox = self.mailbox.clone();
        let on_signal = Box::new(move |signal| mailbox.post(Event::Key(signal)));
        self.show(Shown::RecordingKeys(String::new()));
        match self.parts.desktop.capture_keys(on_signal) {
            Ok(capture) => {
                let bindings = self.settings().shortcuts;
                *self.recording.borrow_mut() = Some(Recording {
                    action,
                    recorder: ShortcutRecorder::new(action, bindings, self.parts.platform),
                    _capture: capture,
                });
                self.show(Shown::ShortcutError(String::new()));
                self.show(Shown::RecordingAction(index));
                self.keep_recording_open();
            }
            Err(error) => self.show(Shown::ShortcutError(error)),
        }
    }

    pub fn stop_recording(&self) {
        self.recording_timeout.stop();
        self.recording.borrow_mut().take();
        self.show(Shown::RecordingAction(-1));
        self.show(Shown::RecordingKeys(String::new()));
        self.show(Shown::ShortcutError(String::new()));
    }

    fn on_key(&self, signal: KeySignal) {
        let KeySignal::Key(event) = signal else {
            self.stop_recording();
            return;
        };
        let mut recording = self.recording.borrow_mut();
        let Some(Recording {
            action, recorder, ..
        }) = recording.as_mut()
        else {
            return;
        };
        let action = *action;
        let step = recorder.on_key(event);
        drop(recording);
        self.keep_recording_open();

        match step {
            RecorderStep::Recording(held) => {
                self.show(Shown::RecordingKeys(held.label(self.parts.platform)));
            }
            RecorderStep::Aborted => self.stop_recording(),
            RecorderStep::Committed(combo) => {
                self.update(|settings| settings.shortcuts.set(action, combo));
                self.stop_recording();
                self.show_shortcuts(&self.settings());
            }
            RecorderStep::Rejected(_, error) => {
                self.show(Shown::RecordingKeys(String::new()));
                self.show(Shown::ShortcutError(error.message()));
            }
        }
    }

    pub fn reset_shortcut(&self, index: i32) {
        let Some(&action) = Action::ALL.get(index as usize) else {
            return;
        };
        let mut bindings = self.settings().shortcuts;
        match reset_to_default(action, &mut bindings, self.parts.platform) {
            Ok(()) => {
                self.update(|settings| settings.shortcuts = bindings);
                self.show(Shown::ShortcutError(String::new()));
                self.show_shortcuts(&self.settings());
            }
            Err(error) => self.show(Shown::ShortcutError(error.message())),
        }
    }

    fn microphone_lines(&self) -> Vec<MicrophoneLine> {
        let (devices, default) = &*self.devices.borrow();
        microphone_lines(
            &self.settings().general.microphones,
            devices,
            default.as_deref(),
        )
    }

    fn show_microphones(&self) {
        self.show(Shown::Microphones(self.microphone_lines()));
        if !self.settings().general.microphones.is_empty() {
            self.show(Shown::MicrophoneAuto(false));
        }
    }

    pub fn set_microphone_auto(&self, auto: bool) {
        let ranking = if auto {
            Vec::new()
        } else {
            self.microphone_lines()
                .into_iter()
                .map(|line| line.id)
                .collect()
        };
        self.update(|settings| settings.general.microphones = ranking);
        self.show(Shown::MicrophoneAuto(auto));
        self.show_microphones();
    }

    pub fn move_microphone(&self, index: i32, by: i32) {
        let mut ranking = self.settings().general.microphones;
        let target = index + by;
        if index >= 0 && target >= 0 && (target as usize) < ranking.len() {
            ranking.swap(index as usize, target as usize);
            self.update(|settings| settings.general.microphones = ranking);
            self.show_microphones();
        }
    }

    pub fn toggle_microphone_test(&self) {
        self.show(Shown::MicrophoneError(String::new()));
        self.show(Shown::MicrophoneLevel(0.0));
        if self.microphone_test.borrow_mut().take().is_some() {
            self.show(Shown::MicrophoneTesting(false));
            return;
        }
        let mailbox = self.mailbox.clone();
        let on_level = Box::new(move |level| mailbox.post(Event::Level(level)));
        let ranking = self.settings().general.microphones;
        match self.parts.desktop.test_microphone(ranking, on_level) {
            Ok(test) => {
                *self.microphone_test.borrow_mut() = Some(test);
                self.show(Shown::MicrophoneTesting(true));
            }
            Err(error) => self.show(Shown::MicrophoneError(error)),
        }
    }

    /// With auto off and nothing picked the setting is still "auto-detect", so the toggle is only
    /// the page's state until a language is picked.
    pub fn set_languages_auto(&self, auto: bool) {
        if auto {
            self.update(|settings| settings.general.languages.clear());
        }
        self.show(Shown::LanguagesAuto(auto));
        self.show_languages(&self.settings());
    }

    pub fn toggle_language(&self, index: i32) {
        let Some(&(code, _)) = LANGUAGES.get(index as usize) else {
            return;
        };
        self.update(|settings| {
            let languages = &mut settings.general.languages;
            if languages.iter().any(|language| language == code) {
                languages.retain(|language| language != code);
            } else {
                languages.push(code.to_owned());
            }
        });
        self.show_languages(&self.settings());
    }

    pub fn add_word(&self, word: &str) {
        self.update(|settings| {
            settings.style.add_word(word);
        });
        self.show_dictionary(&self.settings());
    }

    pub fn remove_word(&self, index: i32) {
        self.update(|settings| {
            let dictionary = &mut settings.style.dictionary;
            if (index as usize) < dictionary.len() {
                dictionary.remove(index as usize);
            }
        });
        self.show_dictionary(&self.settings());
    }

    fn change_replacements(&self, change: Rows<Replacement>) {
        change.clone().apply(&mut self.replacements.borrow_mut());
        self.show(Shown::Replacements(change));
    }

    fn change_snippets(&self, change: Rows<Snippet>) {
        change.clone().apply(&mut self.snippets.borrow_mut());
        self.show(Shown::Snippets(change));
    }

    pub fn add_replacement(&self) {
        self.change_replacements(Rows::Add(Replacement {
            wrong: String::new(),
            right: String::new(),
        }));
    }

    pub fn edit_replacement(&self, index: usize, row: Replacement) {
        self.change_replacements(Rows::One(index, row));
        self.save_replacements();
    }

    pub fn remove_replacement(&self, index: usize) {
        let mut rows = self.replacements.borrow().clone();
        if index >= rows.len() {
            return;
        }
        rows.remove(index);
        self.change_replacements(Rows::All(rows));
        self.save_replacements();
    }

    pub fn add_snippet(&self) {
        self.change_snippets(Rows::Add(Snippet {
            trigger: String::new(),
            text: String::new(),
        }));
    }

    pub fn edit_snippet(&self, index: usize, row: Snippet) {
        self.change_snippets(Rows::One(index, row));
        self.save_snippets();
    }

    pub fn remove_snippet(&self, index: usize) {
        let mut rows = self.snippets.borrow().clone();
        if index >= rows.len() {
            return;
        }
        rows.remove(index);
        self.change_snippets(Rows::All(rows));
        self.save_snippets();
    }

    /// Rows still being typed stay on the page but out of the settings.
    fn save_replacements(&self) {
        let rows = self.replacements.borrow();
        let replacements: Vec<_> = rows
            .iter()
            .filter(|row| !row.wrong.trim().is_empty())
            .cloned()
            .collect();
        drop(rows);
        self.update(|settings| settings.style.replacements = replacements);
    }

    fn save_snippets(&self) {
        let rows = self.snippets.borrow();
        let snippets: Vec<_> = rows
            .iter()
            .filter(|row| !row.trigger.trim().is_empty())
            .cloned()
            .collect();
        drop(rows);
        self.update(|settings| settings.style.snippets = snippets);
    }

    fn store(&self) -> Arc<ModelStore> {
        Arc::clone(&self.model_store.borrow())
    }

    fn catalog_rows(&self, ids: &[String]) -> Vec<i32> {
        let models = &self.parts.catalog.models;
        ids.iter()
            .filter_map(|id| models.iter().position(|model| model.id == *id))
            .map(|row| row as i32)
            .collect()
    }

    fn change_models(&self, change: Rows<ModelRow>) {
        change.clone().apply(&mut self.models.borrow_mut());
        self.show(Shown::Models(change));
    }

    fn row(&self, index: usize) -> ModelRow {
        let store = self.store();
        let model = &self.parts.catalog.models[index];
        let mut row = if store.available() {
            model_row(index, model, store.status(model))
        } else {
            model_row_unavailable(index, model)
        };
        row.used_for = self.used_for(&model.id, row.installed);
        row
    }

    fn used_for(&self, id: &str, installed: bool) -> String {
        let verb = if installed { "Used" } else { "Needed" };
        models_in_use(&self.settings(), &self.parts.catalog)
            .into_iter()
            .find(|(model, _)| model == id)
            .map_or_else(String::new, |(_, stage)| format!("{verb} for {stage}"))
    }

    fn show_folder_missing(&self, missing: bool) {
        self.models_folder_missing.set(missing);
        self.show(Shown::ModelsFolderMissing(missing));
    }

    fn show_models(&self) {
        let store = self.store();
        self.show(Shown::ModelsFolder(
            store.root().to_string_lossy().into_owned(),
        ));
        self.show_folder_missing(!store.available());
        self.show(Shown::ModelsReadOnly(!store.writable()));
        let custom = self.settings().pipeline.models_folder.is_some();
        self.show(Shown::ModelsFolderCustom(custom));
        let rows: Vec<_> = (0..self.parts.catalog.models.len())
            .map(|index| self.row(index))
            .collect();
        self.change_models(Rows::All(rows));
        self.show_model_overview();
    }

    /// What depends on which models are downloaded, beyond their own rows.
    fn show_model_overview(&self) {
        let store = self.store();
        let catalog = &self.parts.catalog;
        let downloaded: Vec<_> = catalog
            .models
            .iter()
            .filter(|model| store.status(model) == ModelStatus::Installed)
            .collect();
        self.show(Shown::DownloadedSummary(downloaded_summary(&downloaded)));
        self.show(Shown::ModelsInUseSummary(self.missing_summary()));
        let menu = |ids: Vec<String>, names: Vec<String>| {
            let entries = ids.iter().zip(names).map(|(id, name)| {
                if downloaded.iter().any(|model| model.id == *id) {
                    format!("✓ {name}")
                } else {
                    name
                }
            });
            entries.collect()
        };
        self.show(Shown::ModelMenus {
            asr: menu(self.choices.asr_ids(), self.choices.asr_names()),
            vad: menu(self.choices.vad_ids(), self.choices.vad_names()),
            llm: menu(self.choices.llm_ids(), self.choices.llm_names()),
        });
    }

    /// The pipeline's models that aren't on disk yet, by their place in the catalog.
    fn missing_models(&self) -> Vec<usize> {
        let store = self.store();
        let in_use = models_in_use(&self.settings(), &self.parts.catalog);
        let catalog = &self.parts.catalog.models;
        (0..catalog.len())
            .filter(|&index| in_use.iter().any(|(id, _)| *id == catalog[index].id))
            .filter(|&index| store.status(&catalog[index]) != ModelStatus::Installed)
            .collect()
    }

    /// Says what the pipeline still needs to download, or nothing once it's all there.
    fn missing_summary(&self) -> String {
        let catalog = &self.parts.catalog.models;
        let missing: Vec<_> = self
            .missing_models()
            .into_iter()
            .map(|index| &catalog[index])
            .collect();
        downloaded_summary(&missing)
    }

    /// One after another, like every model task (ticket 11: no file locking).
    pub fn download_models_in_use(&self) {
        if !self.busy() {
            self.model_queue.replace(self.missing_models().into());
            self.download_next_model();
        }
    }

    fn download_next_model(&self) {
        let next = self.model_queue.borrow_mut().pop_front();
        if let Some(index) = next {
            self.download_model(index as i32);
        }
    }

    /// The pipeline changed: rows keep their task and progress, only their stage moves.
    fn show_models_in_use(&self) {
        for (index, model) in self.parts.catalog.models.iter().enumerate() {
            let row = self.models.borrow().get(index).cloned();
            if let Some(mut row) = row {
                let used_for = self.used_for(&model.id, row.installed);
                if row.used_for != used_for {
                    row.used_for = used_for;
                    self.change_models(Rows::One(index, row));
                }
            }
        }
        self.show(Shown::ModelsInUseSummary(self.missing_summary()));
    }

    pub fn open_models_folder(&self) {
        self.recheck_models_folder();
        let store = self.store();
        match store.prepare() {
            Ok(()) => self.open(&store.root().to_string_lossy()),
            Err(error) => self.show(Shown::ModelError(error.to_string())),
        }
    }

    pub fn open_data_folder(&self) {
        let folder = &self.parts.paths.data_dir;
        match std::fs::create_dir_all(folder) {
            Ok(()) => self.open(&folder.to_string_lossy()),
            Err(error) => self.show(Shown::DataStatus(format!(
                "Couldn't open the folder: {error}"
            ))),
        }
    }

    /// A model task or a move is running. Only one at a time (ticket 11: no file locking).
    fn busy(&self) -> bool {
        self.model_task.get().is_some() || self.moving_models.get()
    }

    pub fn choose_models_folder(&self) {
        if self.busy() {
            return;
        }
        let weak = self.me.clone();
        self.parts.desktop.pick_folder(
            self.store().root(),
            Box::new(move |folder| {
                if let Some(this) = weak.upgrade() {
                    this.offer_folder(Some(folder));
                }
            }),
        );
    }

    /// What the picker came back with, for a front end whose picker runs on its own.
    pub fn folder_picked(&self, folder: PathBuf) {
        self.offer_folder(Some(folder));
    }

    pub fn use_default_models_folder(&self) {
        self.offer_folder(None);
    }

    /// Lets the models folder be checked again, as "Check again" does.
    pub fn recheck_models_folder(&self) {
        // Rebuilding the rows would drop a running task's progress and Cancel.
        if !self.busy() {
            self.show(Shown::ModelError(String::new()));
            self.show_models();
        }
    }

    /// Switches to `folder`, first asking what to do with any models in the current one.
    fn offer_folder(&self, folder: Option<PathBuf>) {
        self.show(Shown::ModelError(String::new()));
        if self.busy() {
            return;
        }
        // The settings file is text, so a path it can't hold wouldn't survive a restart.
        if folder.as_ref().is_some_and(|path| path.to_str().is_none()) {
            self.show(Shown::ModelError(
                "That folder's name can't be saved. Choose another.".into(),
            ));
            return;
        }
        let default_root = self.parts.paths.data_dir.join("models");
        let folder = folder.filter(|path| *path != default_root);
        let target = store_for(&self.parts, folder.as_deref());
        let store = self.store();
        if target.root() == store.root() {
            if self.settings().pipeline.models_folder != folder {
                self.switch_folder(folder);
            }
            return;
        }
        let models = self.models_on_disk();
        if models.is_empty() {
            self.switch_folder(folder);
            return;
        }
        let size: u64 = models.iter().map(ModelEntry::download_size).sum();
        let summary = match models.len() {
            1 => format!("1 model ({}) is in the current folder.", format_bytes(size)),
            count => format!(
                "{count} models ({}) are in the current folder.",
                format_bytes(size)
            ),
        };
        self.show(Shown::PendingSummary(summary));
        self.show(Shown::PendingFolder(
            target.root().to_string_lossy().into_owned(),
        ));
        self.pending_folder.replace(Some(folder));
    }

    /// The models with anything on disk, partial downloads included.
    fn models_on_disk(&self) -> Vec<ModelEntry> {
        let store = self.store();
        self.parts
            .catalog
            .models
            .iter()
            .filter(|model| store.status(model) != ModelStatus::NotInstalled)
            .cloned()
            .collect()
    }

    fn take_pending_folder(&self) -> Option<Option<PathBuf>> {
        self.show(Shown::PendingFolder(String::new()));
        self.show(Shown::PendingSummary(String::new()));
        self.pending_folder.take()
    }

    pub fn cancel_pending_folder(&self) {
        self.take_pending_folder();
    }

    pub fn switch_to_pending_folder(&self) {
        if !self.busy()
            && let Some(folder) = self.take_pending_folder()
        {
            self.switch_folder(folder);
        }
    }

    fn switch_folder(&self, folder: Option<PathBuf>) {
        self.update(|settings| settings.pipeline.models_folder = folder);
        self.reload_store();
        (self.parts.on_change)(&self.settings());
    }

    fn reload_store(&self) {
        self.model_store
            .replace(Arc::new(configured_store(&self.parts)));
        self.show_models();
    }

    pub fn move_to_pending_folder(&self) {
        if self.busy() {
            return;
        }
        let Some(folder) = self.take_pending_folder() else {
            return;
        };
        self.show(Shown::MovingModels(true));
        self.show(Shown::ModelBusy(true));
        self.show(Shown::MoveProgress(0.0));
        self.moving_models.set(true);
        self.cancel_download.store(false, Ordering::Relaxed);
        let cancel = Arc::clone(&self.cancel_download);
        let source = self.store();
        let target = store_for(&self.parts, folder.as_deref());
        let models = self.models_on_disk();
        let settings = Arc::clone(&self.parts.store);
        let mailbox = self.mailbox.clone();
        thread::spawn(move || {
            let mut shown = -1;
            // On this thread so that closing the window can't delete the originals with the
            // settings still pointing at them.
            let commit = || settings.update(|all| all.pipeline.models_folder = folder);
            let outcome = source.move_to(
                &target,
                &models,
                &cancel,
                &mut |progress| {
                    let fraction = progress.downloaded as f32 / progress.total.max(1) as f32;
                    let percent = (fraction * 100.0) as i32;
                    if percent != shown {
                        shown = percent;
                        mailbox.post(Event::MoveProgress(fraction));
                    }
                },
                commit,
            );
            let outcome = match outcome {
                Ok(()) | Err(DownloadError::Cancelled) => Ok(()),
                Err(error @ DownloadError::OriginalsLeft(_)) => Err(error.to_string()),
                Err(error) => Err(format!("Couldn't move the models: {error}")),
            };
            mailbox.post(Event::MoveDone(outcome));
        });
    }

    fn finish_move(&self, outcome: Result<(), String>) {
        self.moving_models.set(false);
        self.show(Shown::MovingModels(false));
        self.show(Shown::ModelBusy(false));
        if let Err(error) = outcome {
            self.show(Shown::ModelError(error));
        }
        self.reload_store();
        (self.parts.on_change)(&self.settings());
    }

    /// Marks the model as being worked on. Only one runs at a time (ticket 11: no file locking).
    fn begin_model_task(&self, index: usize, downloading: bool, status: &str) -> bool {
        if self.busy() {
            return false;
        }
        let Some(mut row) = self.models.borrow().get(index).cloned() else {
            return false;
        };
        row.busy = true;
        row.downloading = downloading;
        row.status = status.into();
        self.change_models(Rows::One(index, row));
        self.model_task.set(Some(index));
        self.show(Shown::ModelBusy(true));
        self.show(Shown::ModelError(String::new()));
        true
    }

    fn finish_model_task(&self, index: usize, outcome: Result<Option<String>, String>) {
        self.model_task.set(None);
        self.show(Shown::ModelBusy(false));
        let mut row = self.row(index);
        match outcome {
            Ok(Some(note)) => row.status = note,
            Ok(None) => {}
            Err(error) => {
                self.model_queue.borrow_mut().clear();
                self.show(Shown::ModelError(error));
            }
        }
        self.change_models(Rows::One(index, row));
        self.show_folder_missing(!self.store().available());
        self.show_model_overview();
        self.download_next_model();
    }

    pub fn download_model(&self, index: i32) {
        let index = index as usize;
        if !self.begin_model_task(index, true, "Starting download…") {
            return;
        }
        self.cancel_download.store(false, Ordering::Relaxed);
        let cancel = Arc::clone(&self.cancel_download);
        let store = self.store();
        let model = self.parts.catalog.models[index].clone();
        let mailbox = self.mailbox.clone();
        thread::spawn(move || {
            let mut shown = -1;
            let outcome = store.download(&model, &cancel, &mut |progress| {
                let fraction = progress.downloaded as f32 / progress.total.max(1) as f32;
                let percent = (fraction * 100.0) as i32;
                if percent != shown {
                    shown = percent;
                    mailbox.post(Event::ModelProgress { index, fraction });
                }
            });
            let outcome = match outcome {
                Ok(()) | Err(DownloadError::Cancelled) => Ok(None),
                Err(error) => Err(format!("{}: {error}", model.name)),
            };
            mailbox.post(Event::ModelDone { index, outcome });
        });
    }

    /// Cancel on a running download, and on whatever waits behind it.
    pub fn cancel_download(&self) {
        self.model_queue.borrow_mut().clear();
        self.cancel_download.store(true, Ordering::Relaxed);
    }

    pub fn cancel_move(&self) {
        self.cancel_download.store(true, Ordering::Relaxed);
    }

    pub fn verify_model(&self, index: i32) {
        let index = index as usize;
        // Missing files on a gone drive would read as a damaged model.
        if !self.busy() && !self.store().available() {
            self.show_models();
            return;
        }
        if !self.begin_model_task(index, false, "Verifying…") {
            return;
        }
        let store = self.store();
        let model = self.parts.catalog.models[index].clone();
        let mailbox = self.mailbox.clone();
        thread::spawn(move || {
            let outcome = match store.verify(&model) {
                Ok(true) => Ok(Some("Verified".to_owned())),
                Ok(false) if !store.available() => {
                    let folder = store.root().to_owned();
                    Err(DownloadError::FolderUnavailable(folder).to_string())
                }
                Ok(false) => Err(format!(
                    "{} is damaged. Delete it and download it again.",
                    model.name
                )),
                Err(error) => Err(format!("{}: {error}", model.name)),
            };
            mailbox.post(Event::ModelDone { index, outcome });
        });
    }

    pub fn delete_model(&self, index: i32) {
        let index = index as usize;
        if self.busy() {
            return;
        }
        let model = &self.parts.catalog.models[index];
        let outcome = self
            .store()
            .delete(model)
            .map(|()| None)
            .map_err(|error| format!("{}: {error}", model.name));
        self.finish_model_task(index, outcome);
    }

    /// Only the catalog's model folders: the chosen folder itself holds the user's other files,
    /// and may be a whole drive. Returns what stayed, for the status line.
    fn delete_models_in_chosen_folder(&self) -> Option<String> {
        self.settings().pipeline.models_folder.as_ref()?;
        let store = self.store();
        let folder = store.root().display();
        if !store.available() {
            return Some(format!("models in {folder}, which wasn't connected"));
        }
        let mut failed = false;
        for model in &self.parts.catalog.models {
            failed |= store.delete(model).is_err();
        }
        failed.then(|| format!("models in {folder} that couldn't be removed"))
    }

    fn open(&self, target: &str) {
        if let Err(error) = self.parts.desktop.open(target) {
            self.notice(format!("Couldn't open {target}: {error}"));
        }
    }

    pub fn check_for_updates(&self) {
        self.open(RELEASES_URL);
    }

    pub fn export_all(&self) {
        let paths = &self.parts.paths;
        let settings = self.parts.store.path();
        let status = match data::export_all(&paths.data_dir, settings, &paths.export_dir) {
            Ok(folder) => format!("Exported to {}", folder.display()),
            Err(error) => format!("Export failed: {error}"),
        };
        self.show(Shown::DataStatus(status));
    }

    pub fn delete_all(&self) {
        if self.busy() {
            self.show(Shown::DataStatus(
                "Wait for the model task to finish, then try again.".into(),
            ));
            return;
        }
        self.take_pending_folder();
        self.parts.desktop.before_delete_all();
        let left = self.delete_models_in_chosen_folder();
        let status = match (data::delete_all(&self.parts.paths.data_dir), left) {
            (Err(error), Some(left)) => format!("Delete failed: {error}. Also left {left}."),
            (Err(error), None) => format!("Delete failed: {error}"),
            (Ok(()), Some(left)) => format!("All data deleted, except {left}."),
            (Ok(()), None) => "All data deleted.".to_owned(),
        };
        self.update(|settings| *settings = Settings::default());
        self.reload_store();
        let general = self.settings().general;
        if let Err(error) = self
            .parts
            .desktop
            .set_launch_at_login(general.launch_at_login)
        {
            self.notice(format!("Couldn't change launch at login: {error}"));
        }
        self.parts
            .desktop
            .set_dock_icon_visible(general.show_in_dock);
        self.show(Shown::MicrophoneAuto(true));
        self.show_everything();
        self.show(Shown::DataStatus(status));
        (self.parts.on_change)(&self.settings());
    }
}

impl Drop for Pages {
    fn drop(&mut self) {
        // A `.part` file stays, so the download resumes from the Models page. A cancelled move
        // leaves the models where they were.
        self.cancel_download.store(true, Ordering::Relaxed);
    }
}

fn store_for(parts: &SettingsParts, chosen: Option<&Path>) -> ModelStore {
    ModelStore::configured(chosen, &parts.paths.data_dir.join("models"))
        .fenced(parts.paths.model_fence.clone())
}

fn configured_store(parts: &SettingsParts) -> ModelStore {
    store_for(parts, parts.store.get().pipeline.models_folder.as_deref())
}
