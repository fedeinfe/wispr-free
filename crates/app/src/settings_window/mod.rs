//! The Settings window (spec §5): created on open, dropped on close. [`SettingsController`] ties
//! the .slint pages to the settings store and the rest of the app. Closures stored in the UI hold
//! only weak references to it, so dropping the controller frees the window.

pub mod data;
mod host;
pub mod rows;
pub mod system;
pub mod values;

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::io;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use slint::{
    ComponentHandle, Model, ModelRc, PlatformError, SharedString, Timer, TimerMode, VecModel, Weak,
};
use wf_audio::InputDevice;
use wf_core::settings::{Replacement, Settings, SettingsStore, Snippet};
use wf_core::shortcut::recorder::{RecorderStep, ShortcutRecorder, reset_to_default};
use wf_core::shortcut::{Action, Platform};
use wf_models::{Catalog, DownloadError, ModelStore};
use wf_platform::keys::KeySignal;

use crate::{MicrophoneRow, ModelRow, Prefs, ReplacementRow, SettingsWindow, SnippetRow};
use rows::{LANGUAGES, MicrophoneLine, language_rows, microphone_lines, model_row, shortcut_rows};
use values::{ModelChoices, apply_values, values_of};

pub use host::{MakeParts, SettingsHost};

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
}

#[derive(Debug, Clone)]
pub struct Paths {
    /// Models, history and meetings. Delete all removes this folder.
    pub data_dir: PathBuf,
    /// Where Export all puts its folder.
    pub export_dir: PathBuf,
}

/// Everything a window is built from.
#[derive(Clone)]
pub struct SettingsParts {
    pub store: Arc<SettingsStore>,
    pub catalog: Arc<Catalog>,
    pub models: Arc<ModelStore>,
    pub desktop: Rc<dyn Desktop>,
    pub paths: Paths,
    pub platform: Platform,
}

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
        recommended: String,
    },
    ModelProgress {
        index: usize,
        fraction: f32,
    },
    ModelDone {
        index: usize,
        outcome: Result<Option<String>, String>,
    },
}

/// Delivers [`Event`]s to the window from any thread. Dropped silently once the window is gone.
#[derive(Clone)]
pub struct Mailbox {
    events: Sender<Event>,
    window: Weak<SettingsWindow>,
}

impl Mailbox {
    pub fn post(&self, event: Event) {
        if self.events.send(event).is_ok() {
            let _ = self.window.upgrade_in_event_loop(|window| {
                window.global::<Prefs>().invoke_events_arrived();
            });
        }
    }
}

struct Recording {
    action: Action,
    recorder: ShortcutRecorder,
    _capture: Box<dyn Any>,
}

/// One open Settings window. Dropping it closes everything the window started.
pub struct SettingsController {
    inner: Rc<Inner>,
}

struct Inner {
    window: SettingsWindow,
    parts: SettingsParts,
    choices: ModelChoices,
    mailbox: Mailbox,
    events: Receiver<Event>,
    recording: RefCell<Option<Recording>>,
    recording_timeout: Timer,
    microphone_test: RefCell<Option<Box<dyn Any>>>,
    devices: RefCell<(Vec<InputDevice>, Option<String>)>,
    models: Rc<VecModel<ModelRow>>,
    model_task: Cell<Option<usize>>,
    cancel_download: Arc<AtomicBool>,
    replacements: Rc<VecModel<ReplacementRow>>,
    snippets: Rc<VecModel<SnippetRow>>,
}

/// Connects a `Prefs` callback to a method of [`Inner`] through a weak reference.
macro_rules! wire {
    ($inner:ident, $prefs:ident . $on:ident, |$this:ident $(, $arg:ident)*| $body:expr) => {{
        let weak = Rc::downgrade($inner);
        $prefs.$on(move |$($arg),*| {
            if let Some($this) = weak.upgrade() {
                $body
            }
        });
    }};
}

impl SettingsController {
    pub fn new(parts: SettingsParts) -> Result<Self, PlatformError> {
        let window = SettingsWindow::new()?;
        let (sender, events) = mpsc::channel();
        let mailbox = Mailbox {
            events: sender,
            window: window.as_weak(),
        };
        let choices = ModelChoices::from_catalog(&parts.catalog);
        let inner = Rc::new(Inner {
            window,
            parts,
            choices,
            mailbox,
            events,
            recording: RefCell::default(),
            recording_timeout: Timer::default(),
            microphone_test: RefCell::default(),
            devices: RefCell::default(),
            models: Rc::default(),
            model_task: Cell::default(),
            cancel_download: Arc::default(),
            replacements: Rc::default(),
            snippets: Rc::default(),
        });
        inner.connect();
        inner.show_everything();
        Ok(Self { inner })
    }

    pub fn window(&self) -> &SettingsWindow {
        &self.inner.window
    }

    pub fn mailbox(&self) -> Mailbox {
        self.inner.mailbox.clone()
    }

    pub fn start_probes(&self) {
        self.inner.parts.desktop.start_probes(&self.inner.mailbox);
    }

    pub fn bring_to_front(&self) {
        self.inner.parts.desktop.bring_to_front();
    }

    pub fn handle(&self, event: Event) {
        self.inner.handle(event);
    }

    /// Re-reads what is on disk for the Models page.
    pub fn refresh_models(&self) {
        self.inner.show_models();
    }
}

impl Drop for SettingsController {
    fn drop(&mut self) {
        // A `.part` file stays, so the download resumes from the Models page.
        self.inner.cancel_download.store(true, Ordering::Relaxed);
    }
}

impl Inner {
    fn prefs(&self) -> Prefs<'_> {
        self.window.global::<Prefs>()
    }

    fn connect(self: &Rc<Self>) {
        let inner = self;
        let prefs = self.prefs();
        prefs.set_is_macos(self.parts.platform == Platform::MacOs);
        prefs.set_version(env!("CARGO_PKG_VERSION").into());
        prefs.set_data_folder(self.parts.paths.data_dir.to_string_lossy().as_ref().into());
        prefs.set_asr_options(string_model(self.choices.asr_names()));
        prefs.set_vad_options(string_model(self.choices.vad_names()));
        prefs.set_llm_options(string_model(self.choices.llm_names()));
        prefs.set_models(ModelRc::from(Rc::clone(&self.models)));
        prefs.set_replacements(ModelRc::from(Rc::clone(&self.replacements)));
        prefs.set_snippets(ModelRc::from(Rc::clone(&self.snippets)));

        wire!(inner, prefs.on_commit, |this| this.commit());
        wire!(inner, prefs.on_events_arrived, |this| this.drain_events());
        wire!(inner, prefs.on_record_shortcut, |this, index| this
            .start_recording(index));
        wire!(inner, prefs.on_stop_recording, |this| this.stop_recording());
        wire!(inner, prefs.on_reset_shortcut, |this, index| this
            .reset_shortcut(index));
        wire!(inner, prefs.on_set_microphone_auto, |this, auto| this
            .set_microphone_auto(auto));
        wire!(inner, prefs.on_move_microphone, |this, index, by| this
            .move_microphone(index, by));
        wire!(inner, prefs.on_toggle_microphone_test, |this| this
            .toggle_microphone_test());
        wire!(inner, prefs.on_set_languages_auto, |this, auto| this
            .set_languages_auto(auto));
        wire!(inner, prefs.on_toggle_language, |this, index| this
            .toggle_language(index));
        wire!(inner, prefs.on_download_model, |this, index| this
            .download_model(index));
        wire!(inner, prefs.on_cancel_download, |this| this
            .cancel_download
            .store(true, Ordering::Relaxed));
        wire!(inner, prefs.on_delete_model, |this, index| this
            .delete_model(index));
        wire!(inner, prefs.on_verify_model, |this, index| this
            .verify_model(index));
        wire!(inner, prefs.on_add_word, |this, word| this.add_word(&word));
        wire!(inner, prefs.on_remove_word, |this, index| this
            .remove_word(index));
        wire!(inner, prefs.on_add_replacement, |this| {
            this.replacements.push(ReplacementRow::default());
        });
        wire!(inner, prefs.on_edit_replacement, |this, index, row| {
            this.replacements.set_row_data(index as usize, row);
            this.save_replacements();
        });
        wire!(inner, prefs.on_remove_replacement, |this, index| {
            let mut rows: Vec<_> = this.replacements.iter().collect();
            rows.remove(index as usize);
            this.replacements.set_vec(rows);
            this.save_replacements();
        });
        wire!(inner, prefs.on_add_snippet, |this| {
            this.snippets.push(SnippetRow::default());
        });
        wire!(inner, prefs.on_edit_snippet, |this, index, row| {
            this.snippets.set_row_data(index as usize, row);
            this.save_snippets();
        });
        wire!(inner, prefs.on_remove_snippet, |this, index| {
            let mut rows: Vec<_> = this.snippets.iter().collect();
            rows.remove(index as usize);
            this.snippets.set_vec(rows);
            this.save_snippets();
        });
        wire!(inner, prefs.on_open_data_folder, |this| this
            .open(&this.parts.paths.data_dir.to_string_lossy()));
        wire!(inner, prefs.on_export_all, |this| this.export_all());
        wire!(inner, prefs.on_delete_all, |this| this.delete_all());
        wire!(inner, prefs.on_check_for_updates, |this| this
            .open(RELEASES_URL));
    }

    fn settings(&self) -> Settings {
        self.parts.store.get()
    }

    /// Changes the settings and reports a failed save at the top of the window.
    fn update(&self, change: impl FnOnce(&mut Settings)) {
        if let Err(error) = self.parts.store.update(change) {
            self.prefs()
                .set_notice(format!("Couldn't save settings: {error}").into());
        }
    }

    fn show_everything(&self) {
        let settings = self.settings();
        self.show_values(&settings);
        self.show_shortcuts(&settings);
        self.show_microphones();
        let prefs = self.prefs();
        prefs.set_languages_auto(settings.general.languages.is_empty());
        prefs.set_languages(ModelRc::new(VecModel::from(language_rows(
            &settings.general.languages,
        ))));
        self.show_dictionary(&settings);
        self.replacements.set_vec(
            settings
                .style
                .replacements
                .iter()
                .map(|replacement| ReplacementRow {
                    wrong: replacement.wrong.as_str().into(),
                    right: replacement.right.as_str().into(),
                })
                .collect::<Vec<_>>(),
        );
        self.snippets.set_vec(
            settings
                .style
                .snippets
                .iter()
                .map(|snippet| SnippetRow {
                    trigger: snippet.trigger.as_str().into(),
                    text: snippet.text.as_str().into(),
                })
                .collect::<Vec<_>>(),
        );
        self.show_models();
        if let Some(path) = self.parts.store.set_aside() {
            prefs.set_notice(
                format!(
                    "The settings file was damaged, so defaults are in use. A copy is at {}.",
                    path.display()
                )
                .into(),
            );
        }
    }

    fn show_values(&self, settings: &Settings) {
        let values = values_of(settings, &self.parts.catalog, &self.choices);
        let prefs = self.prefs();
        if prefs.get_values() != values {
            prefs.set_values(values);
        }
    }

    fn show_shortcuts(&self, settings: &Settings) {
        let rows = shortcut_rows(settings, self.parts.platform);
        self.prefs()
            .set_shortcuts(ModelRc::new(VecModel::from(rows)));
    }

    fn show_dictionary(&self, settings: &Settings) {
        self.prefs()
            .set_dictionary(string_model(settings.style.dictionary.clone()));
    }

    fn commit(&self) {
        let prefs = self.prefs();
        prefs.set_notice(SharedString::new());
        let values = prefs.get_values();
        let before = self.settings();
        self.update(|settings| {
            apply_values(settings, &values, &self.parts.catalog, &self.choices);
        });
        let after = self.settings();

        let wanted_login = after.general.launch_at_login;
        if wanted_login != before.general.launch_at_login
            && let Err(error) = self.parts.desktop.set_launch_at_login(wanted_login)
        {
            prefs.set_notice(format!("Couldn't change launch at login: {error}").into());
            self.update(|settings| settings.general.launch_at_login = !wanted_login);
        }
        if after.general.show_in_dock != before.general.show_in_dock {
            self.parts
                .desktop
                .set_dock_icon_visible(after.general.show_in_dock);
        }
        self.show_values(&self.settings());
    }

    fn drain_events(&self) {
        while let Ok(event) = self.events.try_recv() {
            self.handle(event);
        }
    }

    fn handle(&self, event: Event) {
        let prefs = self.prefs();
        match event {
            Event::Key(signal) => self.on_key(signal),
            Event::Level(level) => {
                if self.microphone_test.borrow().is_some() {
                    prefs.set_microphone_level(level);
                }
            }
            Event::Microphones { devices, default } => {
                *self.devices.borrow_mut() = (devices, default);
                self.show_microphones();
            }
            Event::Hardware {
                summary,
                recommended,
            } => {
                prefs.set_hardware_summary(summary.into());
                prefs.set_recommended_tier(recommended.into());
            }
            Event::ModelProgress { index, fraction } => {
                if let Some(mut row) = self.models.row_data(index) {
                    row.progress = fraction;
                    row.status = format!("Downloading… {:.0}%", fraction * 100.0).into();
                    self.models.set_row_data(index, row);
                }
            }
            Event::ModelDone { index, outcome } => self.finish_model_task(index, outcome),
        }
    }

    fn start_recording(self: &Rc<Self>, index: i32) {
        let prefs = self.prefs();
        let Some(&action) = Action::ALL.get(index as usize) else {
            return;
        };
        let mailbox = self.mailbox.clone();
        let on_signal = Box::new(move |signal| mailbox.post(Event::Key(signal)));
        prefs.set_recording_keys(SharedString::new());
        match self.parts.desktop.capture_keys(on_signal) {
            Ok(capture) => {
                let bindings = self.settings().shortcuts;
                *self.recording.borrow_mut() = Some(Recording {
                    action,
                    recorder: ShortcutRecorder::new(action, bindings, self.parts.platform),
                    _capture: capture,
                });
                prefs.set_shortcut_error(SharedString::new());
                prefs.set_recording_action(index);
                let weak = Rc::downgrade(self);
                self.recording_timeout.start(
                    TimerMode::SingleShot,
                    RECORDING_IDLE_LIMIT,
                    move || {
                        if let Some(this) = weak.upgrade() {
                            this.stop_recording();
                        }
                    },
                );
            }
            Err(error) => prefs.set_shortcut_error(error.into()),
        }
    }

    fn stop_recording(&self) {
        self.recording_timeout.stop();
        self.recording.borrow_mut().take();
        let prefs = self.prefs();
        prefs.set_recording_action(-1);
        prefs.set_recording_keys(SharedString::new());
        prefs.set_shortcut_error(SharedString::new());
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
        self.recording_timeout.restart();

        let prefs = self.prefs();
        match step {
            RecorderStep::Recording(held) => {
                prefs.set_recording_keys(held.label(self.parts.platform).into());
            }
            RecorderStep::Aborted => self.stop_recording(),
            RecorderStep::Committed(combo) => {
                self.update(|settings| settings.shortcuts.set(action, combo));
                self.stop_recording();
                self.show_shortcuts(&self.settings());
            }
            RecorderStep::Rejected(_, error) => {
                prefs.set_recording_keys(SharedString::new());
                prefs.set_shortcut_error(error.message().into());
            }
        }
    }

    fn reset_shortcut(&self, index: i32) {
        let Some(&action) = Action::ALL.get(index as usize) else {
            return;
        };
        let mut bindings = self.settings().shortcuts;
        match reset_to_default(action, &mut bindings, self.parts.platform) {
            Ok(()) => {
                self.update(|settings| settings.shortcuts = bindings);
                self.prefs().set_shortcut_error(SharedString::new());
                self.show_shortcuts(&self.settings());
            }
            Err(error) => self.prefs().set_shortcut_error(error.message().into()),
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
        let rows: Vec<_> = self
            .microphone_lines()
            .into_iter()
            .map(|line| MicrophoneRow {
                name: line.name.into(),
                connected: line.connected,
            })
            .collect();
        let prefs = self.prefs();
        prefs.set_microphones(ModelRc::new(VecModel::from(rows)));
        if !self.settings().general.microphones.is_empty() {
            prefs.set_microphone_auto(false);
        }
    }

    fn set_microphone_auto(&self, auto: bool) {
        let ranking = if auto {
            Vec::new()
        } else {
            self.microphone_lines()
                .into_iter()
                .map(|line| line.id)
                .collect()
        };
        self.update(|settings| settings.general.microphones = ranking);
        self.prefs().set_microphone_auto(auto);
        self.show_microphones();
    }

    fn move_microphone(&self, index: i32, by: i32) {
        let mut ranking = self.settings().general.microphones;
        let target = index + by;
        if index >= 0 && target >= 0 && (target as usize) < ranking.len() {
            ranking.swap(index as usize, target as usize);
            self.update(|settings| settings.general.microphones = ranking);
            self.show_microphones();
        }
    }

    fn toggle_microphone_test(&self) {
        let prefs = self.prefs();
        prefs.set_microphone_error(SharedString::new());
        prefs.set_microphone_level(0.0);
        if self.microphone_test.borrow_mut().take().is_some() {
            prefs.set_microphone_testing(false);
            return;
        }
        let mailbox = self.mailbox.clone();
        let on_level = Box::new(move |level| mailbox.post(Event::Level(level)));
        let ranking = self.settings().general.microphones;
        match self.parts.desktop.test_microphone(ranking, on_level) {
            Ok(test) => {
                *self.microphone_test.borrow_mut() = Some(test);
                prefs.set_microphone_testing(true);
            }
            Err(error) => prefs.set_microphone_error(error.into()),
        }
    }

    /// With auto off and nothing picked the setting is still "auto-detect", so the toggle is only
    /// the page's state until a language is picked.
    fn set_languages_auto(&self, auto: bool) {
        if auto {
            self.update(|settings| settings.general.languages.clear());
        }
        let prefs = self.prefs();
        prefs.set_languages_auto(auto);
        prefs.set_languages(ModelRc::new(VecModel::from(language_rows(
            &self.settings().general.languages,
        ))));
    }

    fn toggle_language(&self, index: i32) {
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
        self.prefs()
            .set_languages(ModelRc::new(VecModel::from(language_rows(
                &self.settings().general.languages,
            ))));
    }

    fn add_word(&self, word: &str) {
        let word = word.trim();
        if word.is_empty() {
            return;
        }
        self.update(|settings| {
            let dictionary = &mut settings.style.dictionary;
            if !dictionary
                .iter()
                .any(|known| known.eq_ignore_ascii_case(word))
            {
                dictionary.push(word.to_owned());
            }
        });
        self.show_dictionary(&self.settings());
    }

    fn remove_word(&self, index: i32) {
        self.update(|settings| {
            let dictionary = &mut settings.style.dictionary;
            if (index as usize) < dictionary.len() {
                dictionary.remove(index as usize);
            }
        });
        self.show_dictionary(&self.settings());
    }

    /// Rows still being typed stay on the page but out of the settings.
    fn save_replacements(&self) {
        let replacements: Vec<_> = self
            .replacements
            .iter()
            .filter(|row| !row.wrong.trim().is_empty())
            .map(|row| Replacement {
                wrong: row.wrong.to_string(),
                right: row.right.to_string(),
            })
            .collect();
        self.update(|settings| settings.style.replacements = replacements);
    }

    fn save_snippets(&self) {
        let snippets: Vec<_> = self
            .snippets
            .iter()
            .filter(|row| !row.trigger.trim().is_empty())
            .map(|row| Snippet {
                trigger: row.trigger.to_string(),
                text: row.text.to_string(),
            })
            .collect();
        self.update(|settings| settings.style.snippets = snippets);
    }

    fn show_models(&self) {
        let rows: Vec<_> = self
            .parts
            .catalog
            .models
            .iter()
            .map(|model| model_row(model, self.parts.models.status(model)))
            .collect();
        self.models.set_vec(rows);
    }

    /// Marks the model as being worked on. Only one runs at a time (ticket 11: no file locking).
    fn begin_model_task(&self, index: usize, downloading: bool, status: &str) -> bool {
        if self.model_task.get().is_some() {
            return false;
        }
        let Some(mut row) = self.models.row_data(index) else {
            return false;
        };
        row.busy = true;
        row.downloading = downloading;
        row.status = status.into();
        self.models.set_row_data(index, row);
        self.model_task.set(Some(index));
        let prefs = self.prefs();
        prefs.set_model_busy(true);
        prefs.set_model_error(SharedString::new());
        true
    }

    fn finish_model_task(&self, index: usize, outcome: Result<Option<String>, String>) {
        self.model_task.set(None);
        let prefs = self.prefs();
        prefs.set_model_busy(false);
        let model = &self.parts.catalog.models[index];
        let mut row = model_row(model, self.parts.models.status(model));
        match outcome {
            Ok(Some(note)) => row.status = note.into(),
            Ok(None) => {}
            Err(error) => prefs.set_model_error(error.into()),
        }
        self.models.set_row_data(index, row);
    }

    fn download_model(&self, index: i32) {
        let index = index as usize;
        if !self.begin_model_task(index, true, "Starting download…") {
            return;
        }
        self.cancel_download.store(false, Ordering::Relaxed);
        let cancel = Arc::clone(&self.cancel_download);
        let store = Arc::clone(&self.parts.models);
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

    fn verify_model(&self, index: i32) {
        let index = index as usize;
        if !self.begin_model_task(index, false, "Verifying…") {
            return;
        }
        let store = Arc::clone(&self.parts.models);
        let model = self.parts.catalog.models[index].clone();
        let mailbox = self.mailbox.clone();
        thread::spawn(move || {
            let outcome = match store.verify(&model) {
                Ok(true) => Ok(Some("Verified".to_owned())),
                Ok(false) => Err(format!(
                    "{} is damaged. Delete it and download it again.",
                    model.name
                )),
                Err(error) => Err(format!("{}: {error}", model.name)),
            };
            mailbox.post(Event::ModelDone { index, outcome });
        });
    }

    fn delete_model(&self, index: i32) {
        let index = index as usize;
        if self.model_task.get().is_some() {
            return;
        }
        let model = &self.parts.catalog.models[index];
        let outcome = self
            .parts
            .models
            .delete(model)
            .map(|()| None)
            .map_err(|error| format!("{}: {error}", model.name));
        self.finish_model_task(index, outcome);
    }

    fn open(&self, target: &str) {
        if let Err(error) = self.parts.desktop.open(target) {
            self.prefs()
                .set_notice(format!("Couldn't open {target}: {error}").into());
        }
    }

    fn export_all(&self) {
        let paths = &self.parts.paths;
        let settings = self.parts.store.path();
        let status = match data::export_all(&paths.data_dir, settings, &paths.export_dir) {
            Ok(folder) => format!("Exported to {}", folder.display()),
            Err(error) => format!("Export failed: {error}"),
        };
        self.prefs().set_data_status(status.into());
    }

    fn delete_all(&self) {
        let prefs = self.prefs();
        if self.model_task.get().is_some() {
            prefs.set_data_status("Wait for the model task to finish, then try again.".into());
            return;
        }
        let status = match data::delete_all(&self.parts.paths.data_dir) {
            Ok(()) => "All data deleted.".to_owned(),
            Err(error) => format!("Delete failed: {error}"),
        };
        self.update(|settings| *settings = Settings::default());
        let general = self.settings().general;
        if let Err(error) = self
            .parts
            .desktop
            .set_launch_at_login(general.launch_at_login)
        {
            prefs.set_notice(format!("Couldn't change launch at login: {error}").into());
        }
        self.parts
            .desktop
            .set_dock_icon_visible(general.show_in_dock);
        prefs.set_microphone_auto(true);
        self.show_everything();
        prefs.set_data_status(status.into());
    }
}

fn string_model(items: Vec<String>) -> ModelRc<SharedString> {
    let items: Vec<SharedString> = items.into_iter().map(Into::into).collect();
    ModelRc::new(VecModel::from(items))
}
