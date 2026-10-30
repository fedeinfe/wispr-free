//! The Settings window driven through its real .slint UI on Slint's testing backend: what the
//! user does on a page ends up in the settings file, and what is in the file shows up on a page.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use i_slint_backend_testing::{
    AccessibleRole, ElementHandle, init_no_event_loop, mock_elapsed_time,
};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalSize, Model, SharedString};
use wf_audio::InputDevice;
use wf_core::settings::{HistoryRetention, Replacement, Settings, SettingsStore, Snippet, Theme};
use wf_core::shortcut::Key::{self, Alt, Char, Ctrl, Esc, Meta, Shift};
use wf_core::shortcut::Side::{Either, Left};
use wf_core::shortcut::{Action, Bindings, Combo, KeyEvent, Platform};
use wf_models::{Catalog, ModelStore};
use wf_platform::keys::KeySignal;
use wispr_free::settings_window::{
    Desktop, Event, Mailbox, Paths, SettingsController, SettingsHost, SettingsParts,
};
use wispr_free::{Prefs, ReplacementRow, SettingsWindow, SnippetRow};

const GENERAL: i32 = 0;
const MODELS: i32 = 1;
const PIPELINE: i32 = 2;
const STYLE: i32 = 3;
const MEETINGS: i32 = 4;
const SOUNDS: i32 = 5;
const PRIVACY: i32 = 6;

#[derive(Default)]
struct FakeDesktop {
    login: Cell<Option<bool>>,
    login_fails: Cell<bool>,
    dock: Cell<Option<bool>>,
    opened: RefCell<Vec<String>>,
    key_capture: RefCell<Option<Rc<()>>>,
    key_capture_fails: Cell<bool>,
    mic_test: RefCell<Option<Rc<()>>>,
    brought_to_front: Cell<u32>,
    probes_started: Cell<u32>,
}

impl Desktop for FakeDesktop {
    fn bring_to_front(&self) {
        self.brought_to_front.set(self.brought_to_front.get() + 1);
    }

    fn start_probes(&self, _mailbox: &Mailbox) {
        self.probes_started.set(self.probes_started.get() + 1);
    }

    fn set_launch_at_login(&self, enabled: bool) -> io::Result<()> {
        if self.login_fails.get() {
            return Err(io::Error::other("denied"));
        }
        self.login.set(Some(enabled));
        Ok(())
    }

    fn set_dock_icon_visible(&self, visible: bool) {
        self.dock.set(Some(visible));
    }

    fn open(&self, target: &str) -> io::Result<()> {
        self.opened.borrow_mut().push(target.to_owned());
        Ok(())
    }

    fn capture_keys(
        &self,
        _on_signal: Box<dyn FnMut(KeySignal) + Send>,
    ) -> Result<Box<dyn Any>, String> {
        if self.key_capture_fails.get() {
            return Err("needs Input Monitoring".to_owned());
        }
        let guard = Rc::new(());
        *self.key_capture.borrow_mut() = Some(Rc::clone(&guard));
        Ok(Box::new(guard))
    }

    fn test_microphone(
        &self,
        _ranking: Vec<String>,
        _on_level: Box<dyn FnMut(f32) + Send>,
    ) -> Result<Box<dyn Any>, String> {
        let guard = Rc::new(());
        *self.mic_test.borrow_mut() = Some(Rc::clone(&guard));
        Ok(Box::new(guard))
    }
}

impl FakeDesktop {
    fn capturing_keys(&self) -> bool {
        self.key_capture
            .borrow()
            .as_ref()
            .is_some_and(|guard| Rc::strong_count(guard) > 1)
    }

    fn testing_microphone(&self) -> bool {
        self.mic_test
            .borrow()
            .as_ref()
            .is_some_and(|guard| Rc::strong_count(guard) > 1)
    }
}

struct Harness {
    dir: tempfile::TempDir,
    desktop: Rc<FakeDesktop>,
    controller: Option<SettingsController>,
}

impl Harness {
    fn new() -> Self {
        init_no_event_loop();
        Self::on_files(tempfile::tempdir().unwrap())
    }

    fn on_files(dir: tempfile::TempDir) -> Self {
        let mut harness = Self {
            dir,
            desktop: Rc::default(),
            controller: None,
        };
        harness.open();
        harness
    }

    fn settings_path(&self) -> PathBuf {
        self.dir.path().join("config").join("settings.toml")
    }

    fn data_dir(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    fn export_dir(&self) -> PathBuf {
        self.dir.path().join("downloads")
    }

    fn parts(&self) -> SettingsParts {
        SettingsParts {
            store: Arc::new(SettingsStore::open(self.settings_path())),
            catalog: Arc::new(Catalog::bundled()),
            models: Arc::new(ModelStore::new(self.data_dir().join("models"))),
            desktop: self.desktop.clone(),
            paths: Paths {
                data_dir: self.data_dir(),
                export_dir: self.export_dir(),
            },
            platform: Platform::MacOs,
        }
    }

    fn open(&mut self) {
        self.controller = Some(SettingsController::new(self.parts()).unwrap());
        // Tall enough for every row: elements scrolled out of view can't be found.
        self.window()
            .window()
            .set_size(LogicalSize::new(880.0, 4000.0));
    }

    /// Like quitting and starting again: only the file carries over.
    fn restart(&mut self) {
        self.controller = None;
        self.open();
    }

    fn controller(&self) -> &SettingsController {
        self.controller.as_ref().unwrap()
    }

    fn window(&self) -> &SettingsWindow {
        self.controller().window()
    }

    fn prefs(&self) -> Prefs<'_> {
        self.window().global::<Prefs>()
    }

    fn saved(&self) -> Settings {
        SettingsStore::open(self.settings_path()).get()
    }

    fn go_to(&self, page: i32) {
        self.window().set_page(page);
    }

    fn element(&self, label: &str) -> ElementHandle {
        // A row's text carries the same label as its control, so skip plain text.
        ElementHandle::find_by_accessible_label(self.window(), label)
            .find(|element| element.accessible_role() != Some(AccessibleRole::Text))
            .unwrap_or_else(|| panic!("no control labelled {label:?}"))
    }

    fn click(&self, label: &str) {
        self.element(label).invoke_accessible_default_action();
    }

    fn checked(&self, label: &str) -> bool {
        self.element(label).accessible_checked().unwrap()
    }

    fn key(&self, event: KeyEvent) {
        self.controller().handle(Event::Key(KeySignal::Key(event)));
    }

    fn tap(&self, keys: &[Key]) {
        for &key in keys {
            self.key(KeyEvent::press(key));
        }
        for &key in keys {
            self.key(KeyEvent::release(key));
        }
    }
}

type Getter = fn(&Settings) -> bool;

#[test]
fn every_toggle_on_every_page_persists_across_a_restart() {
    let cases: &[(i32, &str, Getter)] = &[
        (GENERAL, "Launch at login", |s| s.general.launch_at_login),
        (GENERAL, "Show pill", |s| s.general.show_pill),
        (GENERAL, "Show in Dock", |s| s.general.show_in_dock),
        (PIPELINE, "Noise suppression", |s| s.pipeline.denoise),
        (PIPELINE, "Silence trimming (VAD)", |s| s.pipeline.vad),
        (PIPELINE, "LLM cleanup", |s| s.pipeline.llm_cleanup),
        (PIPELINE, "Stop hands-free after silence", |s| {
            s.pipeline.vad_auto_stop
        }),
        (STYLE, "Add corrected words automatically", |s| {
            s.style.auto_add_corrections
        }),
        (MEETINGS, "Suggest notes when a meeting starts", |s| {
            s.meetings.auto_detect
        }),
        (MEETINGS, "Live transcript", |s| s.meetings.live_transcript),
        (
            MEETINGS,
            "Keep the audio after the transcript is final",
            |s| s.meetings.keep_audio,
        ),
        (MEETINGS, "Refine pass", |s| s.meetings.refine_pass),
        (MEETINGS, "Tell speakers apart", |s| s.meetings.diarization),
        (SOUNDS, "Sounds", |s| s.media.sounds),
        (PRIVACY, "Context awareness", |s| {
            s.privacy.context_awareness
        }),
    ];

    let mut harness = Harness::new();
    for &(page, label, get) in cases {
        harness.go_to(page);
        let before = harness.checked(label);
        assert_eq!(get(&harness.saved()), before, "{label} starts in sync");

        harness.click(label);
        assert_eq!(get(&harness.saved()), !before, "{label} was saved");

        harness.restart();
        harness.go_to(page);
        assert_eq!(harness.checked(label), !before, "{label} after restart");
    }
}

#[test]
fn a_choice_made_in_a_list_persists_across_a_restart() {
    let mut harness = Harness::new();

    let mut values = harness.prefs().get_values();
    values.theme = 2;
    values.history_retention = 2;
    values.meeting_max_length_minutes = 60;
    values.meeting_summary_template = "Decisions first.".into();
    harness.prefs().set_values(values);
    harness.prefs().invoke_commit();

    let saved = harness.saved();
    assert_eq!(saved.general.theme, Theme::Dark);
    assert_eq!(saved.privacy.history_retention, HistoryRetention::Never);
    assert_eq!(saved.meetings.max_length_minutes, 60);
    assert_eq!(saved.meetings.summary_template, "Decisions first.");

    harness.restart();
    let values = harness.prefs().get_values();
    assert_eq!(values.theme, 2);
    assert_eq!(values.history_retention, 2);
    assert_eq!(values.meeting_summary_template, "Decisions first.");
}

#[test]
fn a_stage_toggle_turns_the_tier_on_the_page_to_custom() {
    let harness = Harness::new();
    harness.go_to(PIPELINE);

    harness.click("Noise suppression");

    assert_eq!(harness.prefs().get_values().tier, 3);
}

#[test]
fn picking_a_tier_moves_the_stage_toggles_on_the_page() {
    let harness = Harness::new();
    harness.go_to(PIPELINE);
    assert!(harness.checked("LLM cleanup"));

    let mut values = harness.prefs().get_values();
    values.tier = 0;
    harness.prefs().set_values(values);
    harness.prefs().invoke_commit();

    assert!(!harness.checked("LLM cleanup"));
}

#[test]
fn launch_at_login_reaches_the_os() {
    let harness = Harness::new();
    harness.go_to(GENERAL);

    harness.click("Launch at login");

    assert_eq!(harness.desktop.login.get(), Some(true));
}

#[test]
fn launch_at_login_goes_back_off_when_the_os_refuses() {
    let harness = Harness::new();
    harness.go_to(GENERAL);
    harness.desktop.login_fails.set(true);

    harness.click("Launch at login");

    assert!(!harness.checked("Launch at login"));
    assert!(!harness.saved().general.launch_at_login);
    assert!(harness.prefs().get_notice().contains("denied"));
}

#[test]
fn the_dock_setting_reaches_the_os() {
    let harness = Harness::new();
    harness.go_to(GENERAL);

    harness.click("Show in Dock");

    assert_eq!(harness.desktop.dock.get(), Some(true));
}

#[test]
fn a_recorded_shortcut_is_stored_shown_and_survives_a_restart() {
    let mut harness = Harness::new();
    harness.go_to(GENERAL);
    assert!(!harness.desktop.capturing_keys());

    harness.click("Change Paste last transcript");
    assert!(harness.desktop.capturing_keys());
    assert_eq!(
        harness.prefs().get_recording_action(),
        Action::ALL
            .iter()
            .position(|&a| a == Action::PasteLast)
            .unwrap() as i32
    );

    harness.key(KeyEvent::press(Ctrl(Left)));
    harness.key(KeyEvent::press(Char('J')));
    assert_eq!(harness.prefs().get_recording_keys(), "Left Ctrl+J");
    harness.key(KeyEvent::release(Char('J')));
    harness.key(KeyEvent::release(Ctrl(Left)));

    assert_eq!(harness.prefs().get_recording_action(), -1);
    assert!(!harness.desktop.capturing_keys(), "capture stops at once");
    let expected = Combo::new([Ctrl(Either), Char('J')]);
    assert_eq!(harness.saved().shortcuts.paste_last, expected);

    harness.restart();
    let row = harness.prefs().get_shortcuts().row_data(3).unwrap();
    assert_eq!(row.action, "Paste last transcript");
    assert_eq!(row.combo, "Ctrl+J");
}

#[test]
fn esc_alone_aborts_the_recording_and_keeps_the_binding() {
    let harness = Harness::new();
    harness.go_to(GENERAL);
    harness.click("Change Paste last transcript");

    harness.key(KeyEvent::press(Esc));

    assert_eq!(harness.prefs().get_recording_action(), -1);
    assert!(!harness.desktop.capturing_keys());
    assert_eq!(
        harness.saved().shortcuts,
        Bindings::default_for(Platform::current())
    );
}

#[test]
fn the_stop_button_aborts_the_recording() {
    let harness = Harness::new();
    harness.go_to(GENERAL);
    harness.click("Change Push-to-talk");

    harness.click("Stop recording");

    assert_eq!(harness.prefs().get_recording_action(), -1);
    assert!(!harness.desktop.capturing_keys());
}

#[test]
fn a_refused_combo_shows_why_and_keeps_recording() {
    let harness = Harness::new();
    harness.go_to(GENERAL);
    harness.click("Change Push-to-talk");

    harness.tap(&[Meta(Left), Char('C')]);

    assert_eq!(
        harness.prefs().get_shortcut_error(),
        "Reserved by the system."
    );
    assert!(harness.desktop.capturing_keys());

    harness.tap(&[Shift(Left), Alt(Left), Char('J')]);

    assert_eq!(harness.prefs().get_shortcut_error(), "");
    assert_eq!(harness.prefs().get_recording_action(), -1);
}

#[test]
fn losing_the_key_stream_ends_the_recording() {
    let harness = Harness::new();
    harness.go_to(GENERAL);
    harness.click("Change Push-to-talk");

    harness.controller().handle(Event::Key(KeySignal::Lost));

    assert_eq!(harness.prefs().get_recording_action(), -1);
}

#[test]
fn when_keys_cannot_be_captured_the_page_says_so() {
    let harness = Harness::new();
    harness.go_to(GENERAL);
    harness.desktop.key_capture_fails.set(true);

    harness.click("Change Push-to-talk");

    assert_eq!(harness.prefs().get_recording_action(), -1);
    assert_eq!(
        harness.prefs().get_shortcut_error(),
        "needs Input Monitoring"
    );
}

#[test]
fn reset_puts_a_shortcut_back_on_its_default() {
    let harness = Harness::new();
    harness.go_to(GENERAL);
    harness.click("Change Paste last transcript");
    harness.tap(&[Ctrl(Left), Char('J')]);
    assert_ne!(
        harness.prefs().get_shortcuts().row_data(3).unwrap().combo,
        "Ctrl+Cmd+V"
    );

    harness.click("Reset Paste last transcript");

    assert_eq!(
        harness.prefs().get_shortcuts().row_data(3).unwrap().combo,
        "Ctrl+Cmd+V"
    );
    assert_eq!(
        harness.saved().shortcuts.paste_last,
        Bindings::default_for(Platform::MacOs).paste_last
    );
}

fn microphones() -> Vec<InputDevice> {
    ["usb", "built-in", "bluetooth"]
        .map(|id| InputDevice {
            id: id.to_owned(),
            name: format!("{id} mic"),
        })
        .to_vec()
}

#[test]
fn turning_auto_off_ranks_the_connected_microphones_with_the_default_first() {
    let harness = Harness::new();
    harness.controller().handle(Event::Microphones {
        devices: microphones(),
        default: Some("built-in".to_owned()),
    });

    harness.prefs().invoke_set_microphone_auto(false);

    assert!(!harness.prefs().get_microphone_auto());
    assert_eq!(
        harness.saved().general.microphones,
        ["built-in", "usb", "bluetooth"]
    );
}

#[test]
fn moving_a_microphone_changes_the_ranking_and_it_survives_a_restart() {
    let mut harness = Harness::new();
    harness.controller().handle(Event::Microphones {
        devices: microphones(),
        default: Some("usb".to_owned()),
    });
    harness.prefs().invoke_set_microphone_auto(false);

    harness.prefs().invoke_move_microphone(2, -1);

    assert_eq!(
        harness.saved().general.microphones,
        ["usb", "bluetooth", "built-in"]
    );

    harness.restart();
    assert!(!harness.prefs().get_microphone_auto());
}

#[test]
fn a_ranked_microphone_that_is_unplugged_stays_listed_as_not_connected() {
    let harness = Harness::new();
    harness
        .parts()
        .store
        .update(|s| s.general.microphones = vec!["gone".into(), "usb".into()])
        .unwrap();
    let harness = {
        let dir = harness.dir;
        let mut next = Harness {
            dir,
            desktop: Rc::default(),
            controller: None,
        };
        next.open();
        next
    };

    harness.controller().handle(Event::Microphones {
        devices: microphones(),
        default: None,
    });

    let rows = harness.prefs().get_microphones();
    let first = rows.row_data(0).unwrap();
    assert_eq!(first.name, "gone");
    assert!(!first.connected);
    assert!(rows.row_data(1).unwrap().connected);
    assert_eq!(rows.row_count(), 4);
}

#[test]
fn auto_clears_the_ranking() {
    let harness = Harness::new();
    harness.controller().handle(Event::Microphones {
        devices: microphones(),
        default: None,
    });
    harness.prefs().invoke_set_microphone_auto(false);

    harness.prefs().invoke_set_microphone_auto(true);

    assert!(harness.saved().general.microphones.is_empty());
}

#[test]
fn the_microphone_test_runs_until_stopped_and_shows_the_level() {
    let harness = Harness::new();

    harness.prefs().invoke_toggle_microphone_test();
    assert!(harness.prefs().get_microphone_testing());
    assert!(harness.desktop.testing_microphone());

    harness.controller().handle(Event::Level(0.5));
    assert_eq!(harness.prefs().get_microphone_level(), 0.5);

    harness.prefs().invoke_toggle_microphone_test();
    assert!(!harness.prefs().get_microphone_testing());
    assert!(!harness.desktop.testing_microphone());
    assert_eq!(harness.prefs().get_microphone_level(), 0.0);
}

#[test]
fn picking_languages_persists_and_none_means_auto_detect() {
    let mut harness = Harness::new();
    assert!(harness.prefs().get_languages_auto());

    harness.prefs().invoke_set_languages_auto(false);
    assert!(!harness.prefs().get_languages_auto());
    let rows = harness.prefs().get_languages();
    let italian = (0..rows.row_count())
        .find(|&i| rows.row_data(i).unwrap().name == "Italian")
        .unwrap();
    harness.prefs().invoke_toggle_language(italian as i32);

    assert_eq!(harness.saved().general.languages, ["it"]);

    harness.restart();
    assert!(!harness.prefs().get_languages_auto());
    assert!(
        harness
            .prefs()
            .get_languages()
            .row_data(italian)
            .unwrap()
            .selected
    );

    harness.prefs().invoke_toggle_language(italian as i32);
    assert!(harness.saved().general.languages.is_empty());
}

#[test]
fn dictionary_words_are_trimmed_kept_unique_and_removable() {
    let harness = Harness::new();

    harness.prefs().invoke_add_word("  Slint ".into());
    harness.prefs().invoke_add_word("slint".into());
    harness.prefs().invoke_add_word("   ".into());
    harness.prefs().invoke_add_word("Infelici".into());

    assert_eq!(harness.saved().style.dictionary, ["Slint", "Infelici"]);

    harness.prefs().invoke_remove_word(0);

    assert_eq!(harness.saved().style.dictionary, ["Infelici"]);
    let rows: Vec<SharedString> = harness.prefs().get_dictionary().iter().collect();
    assert_eq!(rows, ["Infelici"]);
}

#[test]
fn a_replacement_is_saved_once_it_has_a_wrong_word() {
    let mut harness = Harness::new();

    harness.prefs().invoke_add_replacement();
    assert_eq!(harness.prefs().get_replacements().row_count(), 1);
    assert!(harness.saved().style.replacements.is_empty());

    harness.prefs().invoke_edit_replacement(
        0,
        ReplacementRow {
            wrong: "wisper".into(),
            right: "Whisper".into(),
        },
    );

    let expected = Replacement {
        wrong: "wisper".into(),
        right: "Whisper".into(),
    };
    assert_eq!(harness.saved().style.replacements, [expected]);

    harness.restart();
    assert_eq!(harness.prefs().get_replacements().row_count(), 1);
    harness.prefs().invoke_remove_replacement(0);
    assert!(harness.saved().style.replacements.is_empty());
}

#[test]
fn a_snippet_is_saved_once_it_has_a_trigger() {
    let mut harness = Harness::new();

    harness.prefs().invoke_add_snippet();
    assert!(harness.saved().style.snippets.is_empty());

    harness.prefs().invoke_edit_snippet(
        0,
        SnippetRow {
            trigger: "my address".into(),
            text: "Via Roma 1".into(),
        },
    );

    let expected = Snippet {
        trigger: "my address".into(),
        text: "Via Roma 1".into(),
    };
    assert_eq!(harness.saved().style.snippets, [expected]);

    harness.restart();
    let row = harness.prefs().get_snippets().row_data(0).unwrap();
    assert_eq!(row.trigger, "my address");
    harness.prefs().invoke_remove_snippet(0);
    assert!(harness.saved().style.snippets.is_empty());
}

#[test]
fn the_models_page_lists_the_catalog_with_size_ram_and_status() {
    let harness = Harness::new();
    harness.go_to(MODELS);

    let rows = harness.prefs().get_models();
    assert_eq!(rows.row_count(), Catalog::bundled().models.len());
    let silero = rows.iter().find(|row| row.name.contains("Silero")).unwrap();
    assert!(silero.detail.contains("2.2 MB"), "{}", silero.detail);
    assert!(silero.detail.contains("MIT"), "{}", silero.detail);
    assert_eq!(silero.status, "Not installed");
    assert!(!silero.installed);
}

#[test]
fn a_model_on_disk_shows_as_installed_and_deleting_it_removes_it() {
    let harness = Harness::new();
    let catalog = Catalog::bundled();
    let (index, model) = catalog
        .models
        .iter()
        .enumerate()
        .find(|(_, model)| model.id == "silero-vad-v5")
        .unwrap();
    let store = ModelStore::new(harness.data_dir().join("models"));
    let dir = store.model_dir(&model.id);
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join(&model.files[0].name);
    fs::write(&file, vec![0; model.files[0].size as usize]).unwrap();

    harness.controller().refresh_models();
    assert!(
        harness
            .prefs()
            .get_models()
            .row_data(index)
            .unwrap()
            .installed
    );

    harness.prefs().invoke_delete_model(index as i32);

    assert!(!file.exists());
    assert!(
        !harness
            .prefs()
            .get_models()
            .row_data(index)
            .unwrap()
            .installed
    );
}

#[test]
fn hardware_shows_up_once_detected() {
    let harness = Harness::new();
    harness.controller().handle(Event::Hardware {
        summary: "Apple M3 · 24 GB".to_owned(),
        recommended: "Balanced".to_owned(),
    });

    assert_eq!(harness.prefs().get_hardware_summary(), "Apple M3 · 24 GB");
    assert_eq!(harness.prefs().get_recommended_tier(), "Balanced");
}

#[test]
fn the_data_folder_opens_with_the_default_app() {
    let harness = Harness::new();

    harness.prefs().invoke_open_data_folder();

    assert_eq!(
        *harness.desktop.opened.borrow(),
        [harness.data_dir().to_string_lossy().to_string()]
    );
}

#[test]
fn check_for_updates_only_opens_a_link() {
    let harness = Harness::new();

    harness.prefs().invoke_check_for_updates();

    let opened = harness.desktop.opened.borrow();
    assert_eq!(opened.len(), 1);
    assert!(opened[0].starts_with("https://github.com/"), "{opened:?}");
}

#[test]
fn export_all_copies_settings_and_user_data_but_not_models() {
    let harness = Harness::new();
    harness.go_to(GENERAL);
    harness.click("Show pill");
    fs::create_dir_all(harness.data_dir().join("meetings/m1")).unwrap();
    fs::write(harness.data_dir().join("meetings/m1/notes.md"), "notes").unwrap();
    fs::create_dir_all(harness.data_dir().join("models/big")).unwrap();
    fs::write(harness.data_dir().join("models/big/model.bin"), "weights").unwrap();

    harness.prefs().invoke_export_all();

    let exports: Vec<_> = fs::read_dir(harness.export_dir())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(exports.len(), 1);
    let export = &exports[0];
    assert_eq!(
        fs::read_to_string(export.join("meetings/m1/notes.md")).unwrap(),
        "notes"
    );
    assert!(export.join("settings.toml").exists());
    assert!(!export.join("models").exists());
    assert!(
        harness
            .prefs()
            .get_data_status()
            .contains(export.to_str().unwrap())
    );
}

#[test]
fn delete_all_removes_the_data_and_returns_to_defaults() {
    let harness = Harness::new();
    harness.go_to(GENERAL);
    harness.click("Show pill");
    harness.click("Launch at login");
    fs::create_dir_all(harness.data_dir().join("models/big")).unwrap();
    fs::write(harness.data_dir().join("models/big/model.bin"), "weights").unwrap();

    harness.prefs().invoke_delete_all();

    assert!(!harness.data_dir().join("models").exists());
    assert_eq!(harness.saved(), Settings::default());
    assert_eq!(harness.desktop.login.get(), Some(false));
    assert!(harness.checked("Show pill"), "the page follows");
}

#[test]
fn a_damaged_settings_file_is_reported() {
    let harness = Harness::new();
    let path = harness.settings_path();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "not [ toml").unwrap();

    let reopened = Harness::on_files(harness.dir);

    assert!(reopened.prefs().get_notice().contains("settings.toml.bad"));
}

#[test]
fn closing_ends_key_capture_and_the_microphone_test_and_frees_the_window() {
    let mut harness = Harness::new();
    harness.go_to(GENERAL);
    harness.click("Change Push-to-talk");
    harness.prefs().invoke_toggle_microphone_test();
    let window = harness.window().as_weak();

    harness.controller = None;

    assert!(window.upgrade().is_none(), "nothing keeps the window alive");
    assert!(!harness.desktop.capturing_keys());
    assert!(!harness.desktop.testing_microphone());
}

fn host_on(harness: &Harness) -> (SettingsHost, Arc<SettingsStore>) {
    let parts = harness.parts();
    let store = Arc::clone(&parts.store);
    (SettingsHost::new(Rc::new(move || parts.clone())), store)
}

#[test]
fn the_window_exists_only_while_open() {
    let harness = Harness::new();
    let (host, _) = host_on(&harness);
    assert!(!host.is_open());

    host.open().unwrap();

    assert!(host.is_open());
    assert_eq!(harness.desktop.brought_to_front.get(), 1);
    assert_eq!(harness.desktop.probes_started.get(), 1);
}

#[test]
fn opening_twice_brings_the_same_window_forward() {
    let harness = Harness::new();
    let (host, _) = host_on(&harness);
    host.open().unwrap();
    let first = host.window().unwrap();

    host.open().unwrap();

    assert_eq!(harness.desktop.brought_to_front.get(), 2);
    assert_eq!(harness.desktop.probes_started.get(), 1, "no second window");
    assert!(first.upgrade().is_some());
}

#[test]
fn closing_the_window_drops_it() {
    let harness = Harness::new();
    let (host, _) = host_on(&harness);
    host.open().unwrap();
    let window = host.window().unwrap();

    window
        .upgrade()
        .unwrap()
        .window()
        .dispatch_event(WindowEvent::CloseRequested);
    mock_elapsed_time(std::time::Duration::from_millis(1));

    assert!(!host.is_open());
    assert!(window.upgrade().is_none());
}

#[test]
fn the_window_can_be_opened_again_after_closing_with_what_was_saved() {
    let harness = Harness::new();
    let (host, store) = host_on(&harness);
    host.open().unwrap();
    store
        .update(|settings| settings.media.sounds = false)
        .unwrap();
    host.close();

    host.open().unwrap();

    let window = host.window().unwrap().upgrade().unwrap();
    assert!(!window.global::<Prefs>().get_values().sounds);
}
