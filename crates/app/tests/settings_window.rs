//! The Settings window driven through its real .slint UI on Slint's testing backend: what the
//! user does on a page ends up in the settings file, and what is in the file shows up on a page.

#[path = "support/memory_cap.rs"]
mod memory_cap;

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use i_slint_backend_testing::{
    AccessibleRole, ElementHandle, init_no_event_loop, mock_elapsed_time,
};
use slint::platform::PointerEventButton;
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalSize, Model, SharedString};
use wf_audio::InputDevice;
use wf_core::settings::{
    HistoryRetention, PipelineTier, Replacement, Settings, SettingsStore, Snippet,
};
use wf_core::shortcut::Key::{self, Alt, Char, Ctrl, Esc, Meta, Shift};
use wf_core::shortcut::Side::{Either, Left};
use wf_core::shortcut::{Action, Bindings, Combo, KeyEvent, Platform};
use wf_models::{Catalog, ModelStore};
use wf_platform::keys::KeySignal;
use wispr_free::settings_window::{
    Desktop, Event, Mailbox, Paths, RECORDING_IDLE_LIMIT, SettingsController, SettingsHost,
    SettingsParts,
};
use wispr_free::{Prefs, ReplacementRow, SettingsWindow, SnippetRow};

/// Well above what these tests use, far below what would hurt.
const MEMORY_LIMIT_MB: u64 = 1024;

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
    folder_to_pick: RefCell<Option<PathBuf>>,
    picker_started_at: RefCell<Option<PathBuf>>,
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

    fn pick_folder(&self, start: &Path, done: Box<dyn FnOnce(PathBuf)>) {
        *self.picker_started_at.borrow_mut() = Some(start.to_owned());
        if let Some(folder) = self.folder_to_pick.take() {
            done(folder);
        }
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
    announced: Rc<RefCell<Vec<Settings>>>,
    controller: Option<SettingsController>,
}

impl Harness {
    fn new() -> Self {
        memory_cap::install(MEMORY_LIMIT_MB);
        init_no_event_loop();
        Self::on_files(tempfile::tempdir().unwrap())
    }

    /// Parakeet on a Custom pipeline: the setup where VAD can be turned off. Whisper keeps it on.
    fn on_parakeet() -> Self {
        let mut harness = Self::new();
        let (_, vad, llm) = balanced_models();
        let store = SettingsStore::open(harness.settings_path());
        store
            .update(|settings| {
                settings.pipeline.tier = PipelineTier::Custom;
                settings.pipeline.asr_model = Some(PARAKEET.to_owned());
                settings.pipeline.vad_model = Some(vad);
                settings.pipeline.llm_model = Some(llm);
            })
            .unwrap();
        harness.restart();
        harness
    }

    fn on_files(dir: tempfile::TempDir) -> Self {
        let mut harness = Self {
            dir,
            desktop: Rc::default(),
            announced: Rc::default(),
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
            desktop: self.desktop.clone(),
            on_change: {
                let announced = Rc::clone(&self.announced);
                Rc::new(move |settings: &Settings| announced.borrow_mut().push(settings.clone()))
            },
            paths: Paths {
                data_dir: self.data_dir(),
                export_dir: self.export_dir(),
                model_fence: None,
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
        (STYLE, "Say \"press enter\" to press Enter", |s| {
            s.style.press_enter_command
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

    let mut harness = Harness::on_parakeet();
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
    values.history_retention = 2;
    values.meeting_max_length_minutes = 60;
    values.meeting_summary_template = "Decisions first.".into();
    harness.prefs().set_values(values);
    harness.prefs().invoke_commit();

    let saved = harness.saved();
    assert_eq!(saved.privacy.history_retention, HistoryRetention::Never);
    assert_eq!(saved.meetings.max_length_minutes, 60);
    assert_eq!(saved.meetings.summary_template, "Decisions first.");

    harness.restart();
    let values = harness.prefs().get_values();
    assert_eq!(values.history_retention, 2);
    assert_eq!(values.meeting_summary_template, "Decisions first.");
}

#[test]
fn a_choice_picked_from_a_dropdown_is_saved() {
    let harness = Harness::new();
    harness.go_to(PRIVACY);

    harness
        .element("History")
        .mock_single_click(PointerEventButton::Left);
    let option = ElementHandle::find_by_accessible_label(harness.window(), "Never store")
        .next()
        .expect("the list is open");
    option.mock_single_click(PointerEventButton::Left);

    assert_eq!(
        harness.saved().privacy.history_retention,
        HistoryRetention::Never
    );
    assert_eq!(harness.prefs().get_values().history_retention, 2);
}

#[test]
fn text_typed_into_a_field_is_saved() {
    let harness = Harness::new();
    harness.go_to(MEETINGS);

    harness
        .element("Summary template")
        .set_accessible_value("Decisions first.");

    assert_eq!(
        harness.saved().meetings.summary_template,
        "Decisions first."
    );
}

#[test]
fn text_typed_into_a_replacement_row_is_saved() {
    let harness = Harness::new();
    harness.go_to(STYLE);
    harness.prefs().invoke_add_replacement();

    harness.element("Wrong").set_accessible_value("wisper");
    harness.element("Right").set_accessible_value("Whisper");

    let expected = Replacement {
        wrong: "wisper".into(),
        right: "Whisper".into(),
    };
    assert_eq!(harness.saved().style.replacements, [expected]);
}

#[test]
fn a_stepper_changes_a_number_within_its_limits() {
    let harness = Harness::new();
    harness.go_to(MEETINGS);

    harness
        .element("Maximum length")
        .invoke_accessible_increment_action();
    assert_eq!(harness.saved().meetings.max_length_minutes, 255);

    harness.go_to(PIPELINE);
    let silence = harness.element("Silence before stopping");
    silence.invoke_accessible_increment_action();
    assert_eq!(
        harness.saved().pipeline.vad_auto_stop_seconds,
        3,
        "off until the auto-stop is on"
    );

    harness.click("Stop hands-free after silence");
    silence.invoke_accessible_increment_action();
    assert_eq!(harness.saved().pipeline.vad_auto_stop_seconds, 4);
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
fn the_rest_of_the_app_hears_about_a_change() {
    let harness = Harness::new();
    harness.go_to(SOUNDS);

    harness.click("Sounds");

    let announced = harness.announced.borrow();
    assert!(!announced.last().expect("announced").media.sounds);
}

#[test]
fn the_rest_of_the_app_hears_about_delete_all() {
    let harness = Harness::new();
    harness.go_to(SOUNDS);
    harness.click("Sounds");

    harness.prefs().invoke_delete_all();

    let announced = harness.announced.borrow();
    assert!(announced.last().expect("announced").media.sounds);
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
fn a_forgotten_recording_gives_the_keyboard_back() {
    let harness = Harness::new();
    harness.go_to(GENERAL);
    harness.click("Change Push-to-talk");

    mock_elapsed_time(RECORDING_IDLE_LIMIT - Duration::from_secs(1));
    harness.key(KeyEvent::press(Ctrl(Left)));
    harness.key(KeyEvent::release(Char('Q')));
    mock_elapsed_time(RECORDING_IDLE_LIMIT - Duration::from_secs(1));
    assert!(harness.desktop.capturing_keys(), "a key restarts the wait");

    mock_elapsed_time(Duration::from_secs(2));

    assert!(!harness.desktop.capturing_keys());
    assert_eq!(harness.prefs().get_recording_action(), -1);
    assert_eq!(
        harness.saved().shortcuts,
        Bindings::default_for(Platform::current())
    );
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
            announced: Rc::default(),
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
fn a_word_learned_with_the_window_open_shows_up_and_can_be_removed() {
    let mut harness = Harness::new();
    let parts = harness.parts();
    let store = Arc::clone(&parts.store);
    harness.controller = Some(SettingsController::new(parts).unwrap());

    store
        .update(|settings| {
            settings.style.add_word("Kubernetes");
        })
        .unwrap();
    harness.controller().refresh();

    let rows: Vec<SharedString> = harness.prefs().get_dictionary().iter().collect();
    assert_eq!(rows, ["Kubernetes"]);

    harness.prefs().invoke_remove_word(0);

    assert!(store.get().style.dictionary.is_empty());
    assert_eq!(harness.prefs().get_dictionary().row_count(), 0);
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
    assert!(
        silero.detail.contains("about 30 MB RAM"),
        "{}",
        silero.detail
    );
    assert!(silero.detail.contains("MIT"), "{}", silero.detail);
    assert_eq!(silero.status, "Not downloaded");
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
        recommended: PipelineTier::Balanced,
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
    mock_elapsed_time(Duration::from_millis(1));

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

/// The bundled catalog's smallest model, faked on disk under `root`.
fn fake_model(root: &Path) -> (usize, PathBuf) {
    let catalog = Catalog::bundled();
    let (index, model) = catalog
        .models
        .iter()
        .enumerate()
        .find(|(_, model)| model.id == "silero-vad-v5")
        .unwrap();
    let store = ModelStore::new(root);
    let dir = store.model_dir(&model.id);
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join(&model.files[0].name);
    fs::write(&file, vec![0; model.files[0].size as usize]).unwrap();
    (index, file)
}

impl Harness {
    fn default_models(&self) -> PathBuf {
        self.data_dir().join("models")
    }

    /// A folder standing in for an external drive.
    fn drive(&self) -> PathBuf {
        let drive = self.dir.path().join("ssd");
        fs::create_dir_all(&drive).unwrap();
        drive
    }

    fn pick(&self, folder: &Path) {
        self.desktop.folder_to_pick.replace(Some(folder.to_owned()));
        self.prefs().invoke_choose_models_folder();
    }

    fn row_installed(&self, index: usize) -> bool {
        self.prefs().get_models().row_data(index).unwrap().installed
    }

    fn wait_for_move(&self) {
        self.wait_for_model_task();
    }

    fn wait_for_model_task(&self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.prefs().get_model_busy() {
            assert!(Instant::now() < deadline, "the task never finished");
            self.prefs().invoke_events_arrived();
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[test]
fn a_new_folder_applies_at_once_and_survives_a_restart() {
    let mut harness = Harness::new();
    let drive = harness.drive();
    let (index, _) = fake_model(&drive);
    assert!(!harness.row_installed(index));

    harness.pick(&drive);

    assert!(harness.row_installed(index), "models there are found");
    assert_eq!(
        harness.prefs().get_models_folder(),
        drive.to_string_lossy().as_ref()
    );
    assert!(harness.prefs().get_models_folder_custom());
    assert_eq!(harness.saved().pipeline.models_folder, Some(drive.clone()));
    assert!(
        harness
            .announced
            .borrow()
            .last()
            .unwrap()
            .pipeline
            .models_folder
            .is_some()
    );

    harness.restart();

    assert!(harness.row_installed(index));
    assert_eq!(
        harness.prefs().get_models_folder(),
        drive.to_string_lossy().as_ref()
    );
}

#[test]
fn the_picker_opens_at_the_current_folder() {
    let harness = Harness::new();

    harness.prefs().invoke_choose_models_folder();

    assert_eq!(
        harness.desktop.picker_started_at.borrow().as_deref(),
        Some(harness.default_models().as_path())
    );
    assert_eq!(
        harness.saved().pipeline.models_folder,
        None,
        "cancelling changes nothing"
    );
}

#[test]
fn an_unplugged_drive_is_reported_and_nothing_lands_on_the_wrong_disk() {
    let mut harness = Harness::new();
    let drive = harness.drive();
    let (index, _) = fake_model(&drive);
    harness.pick(&drive);
    let unplugged = harness.dir.path().join("ssd-unplugged");
    fs::rename(&drive, &unplugged).unwrap();

    harness.restart();

    let prefs = harness.prefs();
    assert!(prefs.get_models_folder_missing());
    let row = prefs.get_models().row_data(index).unwrap();
    assert_eq!(row.status, "Folder not connected");
    assert!(!row.installed);
    assert_eq!(
        harness.saved().pipeline.models_folder,
        Some(drive.clone()),
        "the choice is kept"
    );

    prefs.invoke_download_model(index as i32);
    harness.wait_for_model_task();
    assert!(!drive.exists(), "no stand-in folder");

    fs::rename(&unplugged, &drive).unwrap();
    prefs.invoke_recheck_models_folder();

    assert!(!prefs.get_models_folder_missing());
    assert!(harness.row_installed(index));
}

#[test]
fn models_on_disk_ask_first_and_cancelling_changes_nothing() {
    let harness = Harness::new();
    let drive = harness.drive();
    let (index, file) = fake_model(&harness.default_models());
    harness.controller().refresh_models();

    harness.pick(&drive);

    let prefs = harness.prefs();
    assert_eq!(prefs.get_pending_folder(), drive.to_string_lossy().as_ref());
    assert!(
        prefs.get_pending_summary().starts_with("1 model"),
        "{}",
        prefs.get_pending_summary()
    );
    assert_eq!(harness.saved().pipeline.models_folder, None);

    prefs.invoke_cancel_pending_folder();

    assert_eq!(prefs.get_pending_folder(), "");
    assert!(harness.row_installed(index));
    assert!(file.exists());
}

#[test]
fn moving_takes_the_models_to_the_new_folder() {
    let harness = Harness::new();
    let drive = harness.drive();
    let (index, file) = fake_model(&harness.default_models());
    harness.controller().refresh_models();
    harness.pick(&drive);

    harness.prefs().invoke_move_to_pending_folder();
    assert!(
        harness.prefs().get_model_busy(),
        "no downloads during a move"
    );
    harness.wait_for_move();

    let moved = drive.join(file.strip_prefix(harness.default_models()).unwrap());
    assert!(moved.exists());
    assert!(!file.exists());
    assert!(harness.row_installed(index));
    assert!(!harness.prefs().get_model_busy());
    assert_eq!(harness.prefs().get_model_error(), "");
    assert_eq!(harness.saved().pipeline.models_folder, Some(drive));
}

#[test]
fn switching_without_moving_leaves_the_models_where_they_are() {
    let harness = Harness::new();
    let drive = harness.drive();
    let (index, file) = fake_model(&harness.default_models());
    harness.controller().refresh_models();
    harness.pick(&drive);

    harness.prefs().invoke_switch_to_pending_folder();

    assert!(file.exists());
    assert!(!harness.row_installed(index));
    assert_eq!(harness.saved().pipeline.models_folder, Some(drive));
}

#[test]
fn going_back_to_the_default_folder_works_the_same_way() {
    let harness = Harness::new();
    let drive = harness.drive();
    let (index, file) = fake_model(&drive);
    harness.pick(&drive);

    harness.prefs().invoke_use_default_models_folder();
    harness.prefs().invoke_move_to_pending_folder();
    harness.wait_for_move();

    assert!(!file.exists());
    assert!(harness.row_installed(index));
    assert_eq!(harness.saved().pipeline.models_folder, None);
    assert!(!harness.prefs().get_models_folder_custom());
}

#[test]
fn picking_the_current_folder_again_does_nothing() {
    let harness = Harness::new();
    let drive = harness.drive();
    let (index, _) = fake_model(&drive);
    harness.pick(&drive);

    harness.pick(&drive);

    assert_eq!(harness.prefs().get_pending_folder(), "");
    assert!(harness.row_installed(index));
}

#[test]
fn delete_all_removes_models_from_a_chosen_folder_but_not_the_folder() {
    let harness = Harness::new();
    let drive = harness.drive();
    let (_, file) = fake_model(&drive);
    let photo = drive.join("holiday.jpg");
    fs::write(&photo, "precious").unwrap();
    harness.pick(&drive);

    harness.prefs().invoke_delete_all();

    assert!(!file.exists());
    assert_eq!(fs::read(&photo).unwrap(), b"precious");
    assert_eq!(harness.saved().pipeline.models_folder, None);
    assert_eq!(
        harness.prefs().get_models_folder(),
        harness.default_models().to_string_lossy().as_ref()
    );
}

#[test]
fn delete_all_says_when_a_chosen_folder_was_out_of_reach() {
    let mut harness = Harness::new();
    let drive = harness.drive();
    harness.pick(&drive);
    fs::remove_dir(&drive).unwrap();
    harness.restart();

    harness.prefs().invoke_delete_all();

    let status = harness.prefs().get_data_status();
    assert!(status.contains("wasn't connected"), "{status}");
}

impl Harness {
    fn has_control(&self, label: &str) -> bool {
        ElementHandle::find_by_accessible_label(self.window(), label)
            .any(|element| element.accessible_role() != Some(AccessibleRole::Text))
    }

    fn names(rows: slint::ModelRc<wispr_free::ModelRow>) -> Vec<String> {
        rows.iter().map(|row| row.name.to_string()).collect()
    }
}

fn model_name(id: &str) -> String {
    Catalog::bundled().model(id).unwrap().name.clone()
}

/// What the default tier asks for, read from the catalog so a new pick doesn't break the tests.
const PARAKEET: &str = "parakeet-tdt-0.6b-v3-int8";

fn balanced_models() -> (String, String, String) {
    let catalog = Catalog::bundled();
    let preset = catalog.preset(wf_models::Tier::Balanced);
    (
        preset.asr_model.clone(),
        preset.vad_model.clone().unwrap(),
        preset.llm_model.clone().unwrap(),
    )
}

#[test]
fn open_folder_works_before_anything_is_downloaded() {
    let harness = Harness::new();
    assert!(!harness.default_models().exists());

    harness.prefs().invoke_open_models_folder();

    assert!(
        harness.default_models().is_dir(),
        "the file manager needs it to exist"
    );
    assert_eq!(
        harness.desktop.opened.borrow().last().unwrap(),
        harness.default_models().to_string_lossy().as_ref(),
    );
}

#[test]
fn the_models_page_separates_what_is_downloaded() {
    let harness = Harness::new();
    let total = harness.prefs().get_models().row_count();
    let silero = model_name("silero-vad-v5");
    assert_eq!(harness.prefs().get_downloaded_models().row_count(), 0);
    assert_eq!(harness.prefs().get_available_models().row_count(), total);

    let (index, _) = fake_model(&harness.default_models());
    harness.controller().refresh_models();

    let prefs = harness.prefs();
    assert_eq!(
        Harness::names(prefs.get_downloaded_models()),
        std::slice::from_ref(&silero)
    );
    assert!(!Harness::names(prefs.get_available_models()).contains(&silero));
    assert_eq!(prefs.get_available_models().row_count(), total - 1);
    assert!(
        prefs.get_downloaded_summary().starts_with("1 model"),
        "{}",
        prefs.get_downloaded_summary()
    );

    prefs.invoke_delete_model(index as i32);

    assert_eq!(prefs.get_downloaded_models().row_count(), 0);
    assert!(Harness::names(prefs.get_available_models()).contains(&silero));
}

#[test]
fn rows_keep_their_place_in_the_catalog_when_filtered() {
    let harness = Harness::new();
    let (index, _) = fake_model(&harness.default_models());
    harness.controller().refresh_models();

    let row = harness.prefs().get_downloaded_models().row_data(0).unwrap();

    assert_eq!(row.index as usize, index);
}

#[test]
fn a_downloaded_model_says_which_stage_uses_it() {
    let harness = Harness::on_parakeet();
    let (_, vad, _) = balanced_models();
    let (index, _) = fake_model(&harness.default_models());
    assert_eq!(vad, "silero-vad-v5");
    harness.controller().refresh_models();
    let used_for = |harness: &Harness| {
        harness
            .prefs()
            .get_models()
            .row_data(index)
            .unwrap()
            .used_for
    };
    assert_eq!(used_for(&harness), "Used for voice detection");
    let missing = Catalog::bundled()
        .models
        .iter()
        .position(|model| model.id == PARAKEET)
        .unwrap();
    assert_eq!(
        harness
            .prefs()
            .get_models()
            .row_data(missing)
            .unwrap()
            .used_for,
        "Needed for speech recognition",
    );

    harness.go_to(PIPELINE);
    harness.click("Silence trimming (VAD)");

    assert_eq!(used_for(&harness), "");
}

#[test]
fn the_pipeline_offers_a_download_for_each_missing_model() {
    let harness = Harness::new();
    let (asr, vad, llm) = balanced_models();
    harness.go_to(PIPELINE);

    for id in [&asr, &vad, &llm] {
        let label = format!("Download {}", model_name(id));
        assert!(harness.has_control(&label), "no {label:?}");
    }

    fake_model(&harness.default_models());
    harness.controller().refresh_models();

    assert!(!harness.has_control(&format!("Download {}", model_name(&vad))));
    assert!(harness.has_control(&format!("Download {}", model_name(&asr))));
}

#[test]
fn whisper_on_balanced_keeps_silence_trimming_on() {
    let harness = Harness::new();
    harness.go_to(PIPELINE);

    assert!(harness.checked("Silence trimming (VAD)"));
    harness.click("Silence trimming (VAD)");

    assert!(harness.checked("Silence trimming (VAD)"));
    assert!(harness.saved().pipeline.vad);
}

#[test]
fn a_stage_that_is_off_asks_for_no_download() {
    let harness = Harness::on_parakeet();
    let (_, vad, llm) = balanced_models();
    harness.go_to(PIPELINE);

    harness.click("Silence trimming (VAD)");
    harness.click("LLM cleanup");

    assert!(!harness.has_control(&format!("Download {}", model_name(&vad))));
    assert!(!harness.has_control(&format!("Download {}", model_name(&llm))));
}

#[test]
fn an_unplugged_folder_offers_no_download_in_the_pipeline() {
    let mut harness = Harness::new();
    let (asr, ..) = balanced_models();
    let drive = harness.drive();
    harness.pick(&drive);
    fs::remove_dir(&drive).unwrap();
    harness.restart();
    harness.go_to(PIPELINE);

    assert!(!harness.has_control(&format!("Download {}", model_name(&asr))));
}

#[test]
fn the_model_menus_mark_what_is_downloaded() {
    let harness = Harness::new();
    let silero = model_name("silero-vad-v5");
    assert_eq!(
        harness.prefs().get_vad_options().row_data(0).unwrap(),
        silero.as_str()
    );

    fake_model(&harness.default_models());
    harness.controller().refresh_models();

    assert_eq!(
        harness.prefs().get_vad_options().row_data(0).unwrap(),
        format!("✓ {silero}").as_str(),
    );
}

#[test]
fn each_menu_entry_points_at_its_model() {
    let harness = Harness::new();
    let prefs = harness.prefs();
    let names = Harness::names(prefs.get_models());

    for (options, rows) in [
        (prefs.get_asr_options(), prefs.get_asr_rows()),
        (prefs.get_vad_options(), prefs.get_vad_rows()),
        (prefs.get_llm_options(), prefs.get_llm_rows()),
    ] {
        assert_eq!(options.row_count(), rows.row_count());
        for (option, row) in options.iter().zip(rows.iter()) {
            assert_eq!(option.as_str(), names[row as usize]);
        }
    }
}

#[test]
fn a_folder_switch_waits_for_a_running_task() {
    let harness = Harness::new();
    let drive = harness.drive();
    let (index, _) = fake_model(&harness.default_models());
    harness.controller().refresh_models();
    harness.pick(&drive);
    let prefs = harness.prefs();

    // The task counts as running until its result is read from the mailbox.
    prefs.invoke_verify_model(index as i32);
    prefs.invoke_switch_to_pending_folder();

    assert_eq!(harness.saved().pipeline.models_folder, None);
    assert_eq!(
        prefs.get_pending_folder(),
        drive.to_string_lossy().as_ref(),
        "the question stays"
    );
}

#[test]
fn verifying_on_an_unplugged_drive_says_so_instead_of_calling_the_model_damaged() {
    let harness = Harness::new();
    let drive = harness.drive();
    let (index, _) = fake_model(&drive);
    harness.pick(&drive);
    fs::rename(&drive, harness.dir.path().join("unplugged")).unwrap();

    harness.prefs().invoke_verify_model(index as i32);

    let prefs = harness.prefs();
    assert!(prefs.get_models_folder_missing());
    assert_eq!(prefs.get_model_error(), "");
    assert_eq!(
        prefs.get_models().row_data(index).unwrap().status,
        "Folder not connected"
    );
}

#[test]
fn checking_the_folder_again_clears_the_last_error() {
    let harness = Harness::new();
    harness
        .prefs()
        .set_model_error("Couldn't reach the drive".into());

    harness.prefs().invoke_recheck_models_folder();

    assert_eq!(harness.prefs().get_model_error(), "");
}

#[test]
fn picking_the_default_folder_is_the_same_as_use_default() {
    let harness = Harness::new();
    let drive = harness.drive();
    harness.pick(&drive);

    harness.pick(&harness.default_models());

    assert_eq!(harness.saved().pipeline.models_folder, None);
    assert!(!harness.prefs().get_models_folder_custom());
}

#[test]
fn use_default_works_when_the_saved_folder_already_is_the_default() {
    let mut harness = Harness::new();
    let default = harness.default_models();
    SettingsStore::open(harness.settings_path())
        .update(|settings| settings.pipeline.models_folder = Some(default))
        .unwrap();
    harness.restart();
    assert!(harness.prefs().get_models_folder_custom());

    harness.prefs().invoke_use_default_models_folder();

    assert_eq!(harness.saved().pipeline.models_folder, None);
    assert!(!harness.prefs().get_models_folder_custom());
}

#[cfg(unix)]
#[test]
fn delete_all_names_a_chosen_folder_it_could_not_empty() {
    use std::os::unix::fs::PermissionsExt;

    let harness = Harness::new();
    let drive = harness.drive();
    let (_, file) = fake_model(&drive);
    harness.pick(&drive);
    let locked = file.parent().unwrap();
    fs::set_permissions(locked, fs::Permissions::from_mode(0o555)).unwrap();

    harness.prefs().invoke_delete_all();
    fs::set_permissions(locked, fs::Permissions::from_mode(0o755)).unwrap();

    let status = harness.prefs().get_data_status();
    assert!(
        status.contains(drive.to_string_lossy().as_ref()),
        "{status}"
    );
    assert!(file.exists());
}

#[test]
fn open_folder_on_an_unplugged_drive_offers_the_way_back() {
    let harness = Harness::new();
    let drive = harness.drive();
    harness.pick(&drive);
    let unplugged = harness.dir.path().join("unplugged");
    fs::rename(&drive, &unplugged).unwrap();

    harness.prefs().invoke_open_models_folder();

    assert!(
        harness.prefs().get_models_folder_missing(),
        "Check again must appear"
    );
    assert_ne!(harness.prefs().get_model_error(), "");

    fs::rename(&unplugged, &drive).unwrap();
    harness.prefs().invoke_open_models_folder();

    assert!(!harness.prefs().get_models_folder_missing());
    assert_eq!(harness.prefs().get_model_error(), "");
}

#[cfg(unix)]
#[test]
fn delete_all_still_names_the_chosen_folder_when_everything_fails() {
    use std::os::unix::fs::PermissionsExt;

    let mut harness = Harness::new();
    let drive = harness.drive();
    harness.pick(&drive);
    fs::remove_dir(&drive).unwrap();
    harness.restart();
    let locked = harness.data_dir().join("meetings");
    fs::create_dir_all(&locked).unwrap();
    fs::write(locked.join("notes.md"), "notes").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();

    harness.prefs().invoke_delete_all();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

    let status = harness.prefs().get_data_status();
    assert!(status.starts_with("Delete failed"), "{status}");
    assert!(
        status.contains(drive.to_string_lossy().as_ref()),
        "{status}"
    );
}
