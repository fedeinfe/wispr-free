//! The Settings pages with no window: what the user does on a page ends up in the settings file,
//! and what is in the file shows up on a page. The labels are the ones the pages show.

#[path = "support/clock.rs"]
mod clock;
#[path = "support/memory_cap.rs"]
mod memory_cap;
#[path = "support/seen.rs"]
mod seen;

use std::any::Any;
use std::cell::{Cell, Ref, RefCell};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::{Duration, Instant};

use wf_audio::InputDevice;
use wf_core::settings::{
    HistoryRetention, PipelineTier, Replacement, Settings, SettingsStore, Snippet, Theme,
};
use wf_core::shortcut::Key::{self, Alt, Char, Ctrl, Esc, Meta, Shift};
use wf_core::shortcut::Side::{Either, Left};
use wf_core::shortcut::{Action, Bindings, Combo, KeyEvent, Platform};
use wf_models::{Catalog, ModelStore};
use wf_platform::keys::KeySignal;
use wf_shell::settings::Event;
use wf_shell::settings::rows::ModelRow;
use wf_shell::settings::values::SettingsValues;
use wf_shell::settings::{Desktop, Mailbox, Pages, Paths, RECORDING_IDLE_LIMIT, SettingsParts};

use clock::ManualClock;
use seen::Seen;

/// Well above what these tests use, far below what would hurt.
const MEMORY_LIMIT_MB: u64 = 1024;

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

/// The setting behind a toggle, by the label on it.
fn toggle(label: &str) -> fn(&mut SettingsValues) -> &mut bool {
    match label {
        "Launch at login" => |v| &mut v.launch_at_login,
        "Show pill" => |v| &mut v.show_pill,
        "Show in Dock" => |v| &mut v.show_in_dock,
        "Noise suppression" => |v| &mut v.denoise,
        "Silence trimming (VAD)" => |v| &mut v.vad,
        "LLM cleanup" => |v| &mut v.llm_cleanup,
        "Stop hands-free after silence" => |v| &mut v.vad_auto_stop,
        "Add corrected words automatically" => |v| &mut v.auto_add_corrections,
        "Say \"press enter\" to press Enter" => |v| &mut v.press_enter_command,
        "Suggest notes when a meeting starts" => |v| &mut v.meeting_auto_detect,
        "Live transcript" => |v| &mut v.meeting_live_transcript,
        "Keep the audio after the transcript is final" => |v| &mut v.meeting_keep_audio,
        "Refine pass" => |v| &mut v.meeting_refine_pass,
        "Tell speakers apart" => |v| &mut v.meeting_diarization,
        "Sounds" => |v| &mut v.sounds,
        "Context awareness" => |v| &mut v.context_awareness,
        _ => panic!("no toggle labelled {label:?}"),
    }
}

struct Harness {
    dir: tempfile::TempDir,
    desktop: Rc<FakeDesktop>,
    announced: Rc<RefCell<Vec<Settings>>>,
    seen: Rc<RefCell<Seen>>,
    clock: ManualClock,
    pages: Option<Rc<Pages>>,
}

impl Harness {
    fn new() -> Self {
        memory_cap::install(MEMORY_LIMIT_MB);
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
            seen: Rc::default(),
            clock: ManualClock::default(),
            pages: None,
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
            },
            platform: Platform::MacOs,
        }
    }

    fn open_on(&mut self, parts: SettingsParts) {
        *self.seen.borrow_mut() = Seen::new();
        let seen = Rc::clone(&self.seen);
        let view = Rc::new(move |shown| seen.borrow_mut().take(shown));
        self.pages = Some(Pages::new(parts, view, &self.clock, Arc::new(|| {})));
    }

    fn open(&mut self) {
        self.open_on(self.parts());
    }

    /// Like quitting and starting again: only the file carries over.
    fn restart(&mut self) {
        self.pages = None;
        self.open();
    }

    fn pages(&self) -> &Rc<Pages> {
        self.pages.as_ref().unwrap()
    }

    fn seen(&self) -> Ref<'_, Seen> {
        self.seen.borrow()
    }

    fn saved(&self) -> Settings {
        SettingsStore::open(self.settings_path()).get()
    }

    /// Commits what the pages hold, as every control does after editing it.
    fn edit(&self, change: impl FnOnce(&mut SettingsValues)) {
        let mut values = self.seen().values.clone();
        change(&mut values);
        self.pages().commit(&values);
    }

    fn shortcut_row(&self, label: &str) -> i32 {
        let rows = self.seen().shortcuts.clone();
        rows.iter().position(|row| row.action == label).unwrap() as i32
    }

    fn click(&self, label: &str) {
        if let Some(action) = label.strip_prefix("Change ") {
            self.pages().record_shortcut(self.shortcut_row(action));
        } else if let Some(action) = label.strip_prefix("Reset ") {
            self.pages().reset_shortcut(self.shortcut_row(action));
        } else if label == "Stop recording" {
            self.pages().stop_recording();
        } else if label == "Silence trimming (VAD)" && self.seen().vad_forced {
            // The page shows a switch that can't be moved.
        } else {
            let field = toggle(label);
            self.edit(|values| *field(values) = !*field(values));
        }
    }

    fn checked(&self, label: &str) -> bool {
        let mut values = self.seen().values.clone();
        if label == "Silence trimming (VAD)" && self.seen().vad_forced {
            return true;
        }
        *toggle(label)(&mut values)
    }

    fn key(&self, event: KeyEvent) {
        self.pages().handle(Event::Key(KeySignal::Key(event)));
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
    let cases: &[(&str, Getter)] = &[
        ("Launch at login", |s| s.general.launch_at_login),
        ("Show pill", |s| s.general.show_pill),
        ("Show in Dock", |s| s.general.show_in_dock),
        ("Noise suppression", |s| s.pipeline.denoise),
        ("Silence trimming (VAD)", |s| s.pipeline.vad),
        ("LLM cleanup", |s| s.pipeline.llm_cleanup),
        ("Stop hands-free after silence", |s| {
            s.pipeline.vad_auto_stop
        }),
        ("Add corrected words automatically", |s| {
            s.style.auto_add_corrections
        }),
        ("Say \"press enter\" to press Enter", |s| {
            s.style.press_enter_command
        }),
        ("Suggest notes when a meeting starts", |s| {
            s.meetings.auto_detect
        }),
        ("Live transcript", |s| s.meetings.live_transcript),
        ("Keep the audio after the transcript is final", |s| {
            s.meetings.keep_audio
        }),
        ("Refine pass", |s| s.meetings.refine_pass),
        ("Tell speakers apart", |s| s.meetings.diarization),
        ("Sounds", |s| s.media.sounds),
        ("Context awareness", |s| s.privacy.context_awareness),
    ];

    let mut harness = Harness::on_parakeet();
    for &(label, get) in cases {
        let before = harness.checked(label);
        assert_eq!(get(&harness.saved()), before, "{label} starts in sync");

        harness.click(label);
        assert_eq!(get(&harness.saved()), !before, "{label} was saved");

        harness.restart();
        assert_eq!(harness.checked(label), !before, "{label} after restart");
    }
}

#[test]
fn a_choice_made_in_a_list_persists_across_a_restart() {
    let mut harness = Harness::new();

    harness.edit(|values| {
        values.theme = 2;
        values.history_retention = 2;
        values.meeting_max_length_minutes = 60;
        values.meeting_summary_template = "Decisions first.".into();
    });

    let saved = harness.saved();
    assert_eq!(saved.general.theme, Theme::Dark);
    assert_eq!(saved.privacy.history_retention, HistoryRetention::Never);
    assert_eq!(saved.meetings.max_length_minutes, 60);
    assert_eq!(saved.meetings.summary_template, "Decisions first.");

    harness.restart();
    let values = harness.seen().values.clone();
    assert_eq!(values.theme, 2);
    assert_eq!(values.history_retention, 2);
    assert_eq!(values.meeting_summary_template, "Decisions first.");
}

#[test]
fn a_stage_toggle_turns_the_tier_on_the_page_to_custom() {
    let harness = Harness::new();

    harness.click("Noise suppression");

    assert_eq!(harness.seen().values.tier, 3);
}

#[test]
fn picking_a_tier_moves_the_stage_toggles_on_the_page() {
    let harness = Harness::new();
    assert!(harness.checked("LLM cleanup"));

    harness.edit(|values| {
        values.tier = 0;
    });

    assert!(!harness.checked("LLM cleanup"));
}

#[test]
fn the_rest_of_the_app_hears_about_a_change() {
    let harness = Harness::new();

    harness.click("Sounds");

    let announced = harness.announced.borrow();
    assert!(!announced.last().expect("announced").media.sounds);
}

#[test]
fn the_rest_of_the_app_hears_about_delete_all() {
    let harness = Harness::new();
    harness.click("Sounds");

    harness.pages().delete_all();

    let announced = harness.announced.borrow();
    assert!(announced.last().expect("announced").media.sounds);
}

#[test]
fn launch_at_login_reaches_the_os() {
    let harness = Harness::new();

    harness.click("Launch at login");

    assert_eq!(harness.desktop.login.get(), Some(true));
}

#[test]
fn launch_at_login_goes_back_off_when_the_os_refuses() {
    let harness = Harness::new();
    harness.desktop.login_fails.set(true);

    harness.click("Launch at login");

    assert!(!harness.checked("Launch at login"));
    assert!(!harness.saved().general.launch_at_login);
    assert!(harness.seen().notice.contains("denied"));
}

#[test]
fn the_dock_setting_reaches_the_os() {
    let harness = Harness::new();

    harness.click("Show in Dock");

    assert_eq!(harness.desktop.dock.get(), Some(true));
}

#[test]
fn a_recorded_shortcut_is_stored_shown_and_survives_a_restart() {
    let mut harness = Harness::new();
    assert!(!harness.desktop.capturing_keys());

    harness.click("Change Paste last transcript");
    assert!(harness.desktop.capturing_keys());
    assert_eq!(
        harness.seen().recording_action,
        Action::ALL
            .iter()
            .position(|&a| a == Action::PasteLast)
            .unwrap() as i32
    );

    harness.key(KeyEvent::press(Ctrl(Left)));
    harness.key(KeyEvent::press(Char('J')));
    assert_eq!(harness.seen().recording_keys, "Left Ctrl+J");
    harness.key(KeyEvent::release(Char('J')));
    harness.key(KeyEvent::release(Ctrl(Left)));

    assert_eq!(harness.seen().recording_action, -1);
    assert!(!harness.desktop.capturing_keys(), "capture stops at once");
    let expected = Combo::new([Ctrl(Either), Char('J')]);
    assert_eq!(harness.saved().shortcuts.paste_last, expected);

    harness.restart();
    let row = harness.seen().shortcuts[3].clone();
    assert_eq!(row.action, "Paste last transcript");
    assert_eq!(row.combo, "Ctrl+J");
}

#[test]
fn esc_alone_aborts_the_recording_and_keeps_the_binding() {
    let harness = Harness::new();
    harness.click("Change Paste last transcript");

    harness.key(KeyEvent::press(Esc));

    assert_eq!(harness.seen().recording_action, -1);
    assert!(!harness.desktop.capturing_keys());
    assert_eq!(
        harness.saved().shortcuts,
        Bindings::default_for(Platform::current())
    );
}

#[test]
fn the_stop_button_aborts_the_recording() {
    let harness = Harness::new();
    harness.click("Change Push-to-talk");

    harness.click("Stop recording");

    assert_eq!(harness.seen().recording_action, -1);
    assert!(!harness.desktop.capturing_keys());
}

#[test]
fn a_forgotten_recording_gives_the_keyboard_back() {
    let harness = Harness::new();
    harness.click("Change Push-to-talk");

    harness
        .clock
        .advance(RECORDING_IDLE_LIMIT - Duration::from_secs(1));
    harness.key(KeyEvent::press(Ctrl(Left)));
    harness.key(KeyEvent::release(Char('Q')));
    harness
        .clock
        .advance(RECORDING_IDLE_LIMIT - Duration::from_secs(1));
    assert!(harness.desktop.capturing_keys(), "a key restarts the wait");

    harness.clock.advance(Duration::from_secs(2));

    assert!(!harness.desktop.capturing_keys());
    assert_eq!(harness.seen().recording_action, -1);
    assert_eq!(
        harness.saved().shortcuts,
        Bindings::default_for(Platform::current())
    );
}

#[test]
fn a_refused_combo_shows_why_and_keeps_recording() {
    let harness = Harness::new();
    harness.click("Change Push-to-talk");

    harness.tap(&[Meta(Left), Char('C')]);

    assert_eq!(harness.seen().shortcut_error, "Reserved by the system.");
    assert!(harness.desktop.capturing_keys());

    harness.tap(&[Shift(Left), Alt(Left), Char('J')]);

    assert_eq!(harness.seen().shortcut_error, "");
    assert_eq!(harness.seen().recording_action, -1);
}

#[test]
fn losing_the_key_stream_ends_the_recording() {
    let harness = Harness::new();
    harness.click("Change Push-to-talk");

    harness.pages().handle(Event::Key(KeySignal::Lost));

    assert_eq!(harness.seen().recording_action, -1);
}

#[test]
fn when_keys_cannot_be_captured_the_page_says_so() {
    let harness = Harness::new();
    harness.desktop.key_capture_fails.set(true);

    harness.click("Change Push-to-talk");

    assert_eq!(harness.seen().recording_action, -1);
    assert_eq!(harness.seen().shortcut_error, "needs Input Monitoring");
}

#[test]
fn reset_puts_a_shortcut_back_on_its_default() {
    let harness = Harness::new();
    harness.click("Change Paste last transcript");
    harness.tap(&[Ctrl(Left), Char('J')]);
    assert_ne!(harness.seen().shortcuts[3].combo, "Ctrl+Cmd+V");

    harness.click("Reset Paste last transcript");

    assert_eq!(harness.seen().shortcuts[3].combo, "Ctrl+Cmd+V");
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
    harness.pages().handle(Event::Microphones {
        devices: microphones(),
        default: Some("built-in".to_owned()),
    });

    harness.pages().set_microphone_auto(false);

    assert!(!harness.seen().microphone_auto);
    assert_eq!(
        harness.saved().general.microphones,
        ["built-in", "usb", "bluetooth"]
    );
}

#[test]
fn moving_a_microphone_changes_the_ranking_and_it_survives_a_restart() {
    let mut harness = Harness::new();
    harness.pages().handle(Event::Microphones {
        devices: microphones(),
        default: Some("usb".to_owned()),
    });
    harness.pages().set_microphone_auto(false);

    harness.pages().move_microphone(2, -1);

    assert_eq!(
        harness.saved().general.microphones,
        ["usb", "bluetooth", "built-in"]
    );

    harness.restart();
    assert!(!harness.seen().microphone_auto);
}

#[test]
fn a_ranked_microphone_that_is_unplugged_stays_listed_as_not_connected() {
    let harness = Harness::new();
    harness
        .parts()
        .store
        .update(|s| s.general.microphones = vec!["gone".into(), "usb".into()])
        .unwrap();
    let harness = Harness::on_files(harness.dir);

    harness.pages().handle(Event::Microphones {
        devices: microphones(),
        default: None,
    });

    let rows = harness.seen().microphones.clone();
    let first = &rows[0];
    assert_eq!(first.name, "gone");
    assert!(!first.connected);
    assert!(rows[1].connected);
    assert_eq!(rows.len(), 4);
}

#[test]
fn auto_clears_the_ranking() {
    let harness = Harness::new();
    harness.pages().handle(Event::Microphones {
        devices: microphones(),
        default: None,
    });
    harness.pages().set_microphone_auto(false);

    harness.pages().set_microphone_auto(true);

    assert!(harness.saved().general.microphones.is_empty());
}

#[test]
fn the_microphone_test_runs_until_stopped_and_shows_the_level() {
    let harness = Harness::new();

    harness.pages().toggle_microphone_test();
    assert!(harness.seen().microphone_testing);
    assert!(harness.desktop.testing_microphone());

    harness.pages().handle(Event::Level(0.5));
    assert_eq!(harness.seen().microphone_level, 0.5);

    harness.pages().toggle_microphone_test();
    assert!(!harness.seen().microphone_testing);
    assert!(!harness.desktop.testing_microphone());
    assert_eq!(harness.seen().microphone_level, 0.0);
}

#[test]
fn picking_languages_persists_and_none_means_auto_detect() {
    let mut harness = Harness::new();
    assert!(harness.seen().languages_auto);

    harness.pages().set_languages_auto(false);
    assert!(!harness.seen().languages_auto);
    let rows = harness.seen().languages.clone();
    let italian = rows.iter().position(|row| row.name == "Italian").unwrap();
    harness.pages().toggle_language(italian as i32);

    assert_eq!(harness.saved().general.languages, ["it"]);

    harness.restart();
    assert!(!harness.seen().languages_auto);
    assert!(harness.seen().languages[italian].selected);

    harness.pages().toggle_language(italian as i32);
    assert!(harness.saved().general.languages.is_empty());
}

#[test]
fn dictionary_words_are_trimmed_kept_unique_and_removable() {
    let harness = Harness::new();

    harness.pages().add_word("  Slint ");
    harness.pages().add_word("slint");
    harness.pages().add_word("   ");
    harness.pages().add_word("Infelici");

    assert_eq!(harness.saved().style.dictionary, ["Slint", "Infelici"]);

    harness.pages().remove_word(0);

    assert_eq!(harness.saved().style.dictionary, ["Infelici"]);
    assert_eq!(harness.seen().dictionary, ["Infelici"]);
}

#[test]
fn a_word_learned_with_the_window_open_shows_up_and_can_be_removed() {
    let mut harness = Harness::new();
    let parts = harness.parts();
    let store = Arc::clone(&parts.store);
    harness.open_on(parts);

    store
        .update(|settings| {
            settings.style.add_word("Kubernetes");
        })
        .unwrap();
    harness.pages().refresh();

    assert_eq!(harness.seen().dictionary, ["Kubernetes"]);

    harness.pages().remove_word(0);

    assert!(store.get().style.dictionary.is_empty());
    assert_eq!(harness.seen().dictionary.len(), 0);
}

#[test]
fn a_replacement_is_saved_once_it_has_a_wrong_word() {
    let mut harness = Harness::new();

    harness.pages().add_replacement();
    assert_eq!(harness.seen().replacements.len(), 1);
    assert!(harness.saved().style.replacements.is_empty());

    harness.pages().edit_replacement(
        0,
        Replacement {
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
    assert_eq!(harness.seen().replacements.len(), 1);
    harness.pages().remove_replacement(0);
    assert!(harness.saved().style.replacements.is_empty());
}

#[test]
fn a_snippet_is_saved_once_it_has_a_trigger() {
    let mut harness = Harness::new();

    harness.pages().add_snippet();
    assert!(harness.saved().style.snippets.is_empty());

    harness.pages().edit_snippet(
        0,
        Snippet {
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
    let row = harness.seen().snippets[0].clone();
    assert_eq!(row.trigger, "my address");
    harness.pages().remove_snippet(0);
    assert!(harness.saved().style.snippets.is_empty());
}

#[test]
fn the_models_page_lists_the_catalog_with_size_ram_and_status() {
    let harness = Harness::new();

    let rows = harness.seen().models.clone();
    assert_eq!(rows.len(), Catalog::bundled().models.len());
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
    let handle = fs::File::create(&file).unwrap();
    handle.set_len(model.files[0].size).unwrap();

    harness.pages().refresh_models();
    assert!(harness.seen().models[index].installed);

    harness.pages().delete_model(index as i32);

    assert!(!file.exists());
    assert!(!harness.seen().models[index].installed);
}

#[test]
fn hardware_shows_up_once_detected() {
    let harness = Harness::new();
    harness.pages().handle(Event::Hardware {
        summary: "Apple M3 · 24 GB".to_owned(),
        recommended: "Balanced".to_owned(),
    });

    assert_eq!(harness.seen().hardware_summary, "Apple M3 · 24 GB");
    assert_eq!(harness.seen().recommended_tier, "Balanced");
}

#[test]
fn the_data_folder_opens_with_the_default_app() {
    let harness = Harness::new();

    harness.pages().open_data_folder();

    assert_eq!(
        *harness.desktop.opened.borrow(),
        [harness.data_dir().to_string_lossy().to_string()]
    );
}

#[test]
fn check_for_updates_only_opens_a_link() {
    let harness = Harness::new();

    harness.pages().check_for_updates();

    let opened = harness.desktop.opened.borrow();
    assert_eq!(opened.len(), 1);
    assert!(opened[0].starts_with("https://github.com/"), "{opened:?}");
}

#[test]
fn export_all_copies_settings_and_user_data_but_not_models() {
    let harness = Harness::new();
    harness.click("Show pill");
    fs::create_dir_all(harness.data_dir().join("meetings/m1")).unwrap();
    fs::write(harness.data_dir().join("meetings/m1/notes.md"), "notes").unwrap();
    fs::create_dir_all(harness.data_dir().join("models/big")).unwrap();
    fs::write(harness.data_dir().join("models/big/model.bin"), "weights").unwrap();

    harness.pages().export_all();

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
            .seen()
            .data_status
            .contains(export.to_str().unwrap())
    );
}

#[test]
fn delete_all_removes_the_data_and_returns_to_defaults() {
    let harness = Harness::new();
    harness.click("Show pill");
    harness.click("Launch at login");
    fs::create_dir_all(harness.data_dir().join("models/big")).unwrap();
    fs::write(harness.data_dir().join("models/big/model.bin"), "weights").unwrap();

    harness.pages().delete_all();

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

    assert!(reopened.seen().notice.contains("settings.toml.bad"));
}

#[test]
fn closing_ends_key_capture_and_the_microphone_test_and_frees_the_window() {
    let mut harness = Harness::new();
    harness.click("Change Push-to-talk");
    harness.pages().toggle_microphone_test();
    let pages: Weak<Pages> = Rc::downgrade(harness.pages());

    harness.pages = None;

    assert!(pages.upgrade().is_none(), "nothing keeps the pages alive");
    assert!(!harness.desktop.capturing_keys());
    assert!(!harness.desktop.testing_microphone());
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
    let handle = fs::File::create(&file).unwrap();
    handle.set_len(model.files[0].size).unwrap();
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
        self.pages().choose_models_folder();
    }

    fn row_installed(&self, index: usize) -> bool {
        self.seen().models[index].installed
    }

    fn wait_for_move(&self) {
        self.wait_for_model_task();
    }

    fn wait_for_model_task(&self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.seen().model_busy {
            assert!(Instant::now() < deadline, "the task never finished");
            self.pages().drain_events();
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
        harness.seen().models_folder,
        drive.to_string_lossy().as_ref()
    );
    assert!(harness.seen().models_folder_custom);
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
        harness.seen().models_folder,
        drive.to_string_lossy().as_ref()
    );
}

#[test]
fn the_picker_opens_at_the_current_folder() {
    let harness = Harness::new();

    harness.pages().choose_models_folder();

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

    assert!(harness.seen().models_folder_missing);
    let row = harness.seen().models[index].clone();
    assert_eq!(row.status, "Folder not connected");
    assert!(!row.installed);
    assert_eq!(
        harness.saved().pipeline.models_folder,
        Some(drive.clone()),
        "the choice is kept"
    );

    harness.pages().download_model(index as i32);
    harness.wait_for_model_task();
    assert!(!drive.exists(), "no stand-in folder");

    fs::rename(&unplugged, &drive).unwrap();
    harness.pages().recheck_models_folder();

    assert!(!harness.seen().models_folder_missing);
    assert!(harness.row_installed(index));
}

#[test]
fn models_on_disk_ask_first_and_cancelling_changes_nothing() {
    let harness = Harness::new();
    let drive = harness.drive();
    let (index, file) = fake_model(&harness.default_models());
    harness.pages().refresh_models();

    harness.pick(&drive);

    assert_eq!(
        harness.seen().pending_folder,
        drive.to_string_lossy().as_ref()
    );
    assert!(
        harness.seen().pending_summary.starts_with("1 model"),
        "{}",
        harness.seen().pending_summary
    );
    assert_eq!(harness.saved().pipeline.models_folder, None);

    harness.pages().cancel_pending_folder();

    assert_eq!(harness.seen().pending_folder, "");
    assert!(harness.row_installed(index));
    assert!(file.exists());
}

#[test]
fn moving_takes_the_models_to_the_new_folder() {
    let harness = Harness::new();
    let drive = harness.drive();
    let (index, file) = fake_model(&harness.default_models());
    harness.pages().refresh_models();
    harness.pick(&drive);

    harness.pages().move_to_pending_folder();
    assert!(harness.seen().model_busy, "no downloads during a move");
    harness.wait_for_move();

    let moved = drive.join(file.strip_prefix(harness.default_models()).unwrap());
    assert!(moved.exists());
    assert!(!file.exists());
    assert!(harness.row_installed(index));
    assert!(!harness.seen().model_busy);
    assert_eq!(harness.seen().model_error, "");
    assert_eq!(harness.saved().pipeline.models_folder, Some(drive));
}

#[test]
fn switching_without_moving_leaves_the_models_where_they_are() {
    let harness = Harness::new();
    let drive = harness.drive();
    let (index, file) = fake_model(&harness.default_models());
    harness.pages().refresh_models();
    harness.pick(&drive);

    harness.pages().switch_to_pending_folder();

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

    harness.pages().use_default_models_folder();
    harness.pages().move_to_pending_folder();
    harness.wait_for_move();

    assert!(!file.exists());
    assert!(harness.row_installed(index));
    assert_eq!(harness.saved().pipeline.models_folder, None);
    assert!(!harness.seen().models_folder_custom);
}

#[test]
fn picking_the_current_folder_again_does_nothing() {
    let harness = Harness::new();
    let drive = harness.drive();
    let (index, _) = fake_model(&drive);
    harness.pick(&drive);

    harness.pick(&drive);

    assert_eq!(harness.seen().pending_folder, "");
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

    harness.pages().delete_all();

    assert!(!file.exists());
    assert_eq!(fs::read(&photo).unwrap(), b"precious");
    assert_eq!(harness.saved().pipeline.models_folder, None);
    assert_eq!(
        harness.seen().models_folder,
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

    harness.pages().delete_all();

    let status = harness.seen().data_status.clone();
    assert!(status.contains("wasn't connected"), "{status}");
}

impl Harness {
    /// The Pipeline page offers a download for a model that's in use and not on disk, while its
    /// folder is there.
    fn has_control(&self, label: &str) -> bool {
        let seen = self.seen();
        !seen.models_folder_missing
            && seen.models.iter().any(|row| {
                format!("Download {}", row.name) == label
                    && !row.used_for.is_empty()
                    && !row.installed
            })
    }

    fn names(rows: Vec<ModelRow>) -> Vec<String> {
        rows.iter().map(|row| row.name.clone()).collect()
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

    harness.pages().open_models_folder();

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
    let total = harness.seen().models.len();
    let silero = model_name("silero-vad-v5");
    assert_eq!(harness.seen().downloaded_models().len(), 0);
    assert_eq!(harness.seen().available_models().len(), total);

    let (index, _) = fake_model(&harness.default_models());
    harness.pages().refresh_models();

    assert_eq!(
        Harness::names(harness.seen().downloaded_models()),
        std::slice::from_ref(&silero)
    );
    assert!(!Harness::names(harness.seen().available_models()).contains(&silero));
    assert_eq!(harness.seen().available_models().len(), total - 1);
    assert!(
        harness.seen().downloaded_summary.starts_with("1 model"),
        "{}",
        harness.seen().downloaded_summary
    );

    harness.pages().delete_model(index as i32);

    assert_eq!(harness.seen().downloaded_models().len(), 0);
    assert!(Harness::names(harness.seen().available_models()).contains(&silero));
}

#[test]
fn rows_keep_their_place_in_the_catalog_when_filtered() {
    let harness = Harness::new();
    let (index, _) = fake_model(&harness.default_models());
    harness.pages().refresh_models();

    let row = harness.seen().downloaded_models()[0].clone();

    assert_eq!(row.index as usize, index);
}

#[test]
fn a_downloaded_model_says_which_stage_uses_it() {
    let harness = Harness::on_parakeet();
    let (_, vad, _) = balanced_models();
    let (index, _) = fake_model(&harness.default_models());
    assert_eq!(vad, "silero-vad-v5");
    harness.pages().refresh_models();
    let used_for = |harness: &Harness| harness.seen().models[index].used_for.clone();
    assert_eq!(used_for(&harness), "Used for voice detection");
    let missing = Catalog::bundled()
        .models
        .iter()
        .position(|model| model.id == PARAKEET)
        .unwrap();
    assert_eq!(
        harness.seen().models[missing].used_for,
        "Needed for speech recognition",
    );
    harness.click("Silence trimming (VAD)");

    assert_eq!(used_for(&harness), "");
}

#[test]
fn the_pipeline_offers_a_download_for_each_missing_model() {
    let harness = Harness::new();
    let (asr, vad, llm) = balanced_models();

    for id in [&asr, &vad, &llm] {
        let label = format!("Download {}", model_name(id));
        assert!(harness.has_control(&label), "no {label:?}");
    }

    fake_model(&harness.default_models());
    harness.pages().refresh_models();

    assert!(!harness.has_control(&format!("Download {}", model_name(&vad))));
    assert!(harness.has_control(&format!("Download {}", model_name(&asr))));
}

#[test]
fn whisper_on_balanced_keeps_silence_trimming_on() {
    let harness = Harness::new();

    assert!(harness.checked("Silence trimming (VAD)"));
    harness.click("Silence trimming (VAD)");

    assert!(harness.checked("Silence trimming (VAD)"));
    assert!(harness.saved().pipeline.vad);
}

#[test]
fn a_stage_that_is_off_asks_for_no_download() {
    let harness = Harness::on_parakeet();
    let (_, vad, llm) = balanced_models();

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

    assert!(!harness.has_control(&format!("Download {}", model_name(&asr))));
}

#[test]
fn the_model_menus_mark_what_is_downloaded() {
    let harness = Harness::new();
    let silero = model_name("silero-vad-v5");
    assert_eq!(harness.seen().vad_options[0], silero.as_str());

    fake_model(&harness.default_models());
    harness.pages().refresh_models();

    assert_eq!(
        harness.seen().vad_options[0],
        format!("✓ {silero}").as_str(),
    );
}

#[test]
fn each_menu_entry_points_at_its_model() {
    let harness = Harness::new();
    let seen = harness.seen();
    let names = Harness::names(seen.models.clone());
    let facts = seen.facts.as_ref().unwrap();

    for (options, rows) in [
        (&seen.asr_options, &facts.asr_rows),
        (&seen.vad_options, &facts.vad_rows),
        (&seen.llm_options, &facts.llm_rows),
    ] {
        assert_eq!(options.len(), rows.len());
        for (option, row) in options.iter().zip(rows.iter()) {
            assert_eq!(option.as_str(), names[*row as usize]);
        }
    }
}

#[test]
fn a_folder_switch_waits_for_a_running_task() {
    let harness = Harness::new();
    let drive = harness.drive();
    let (index, _) = fake_model(&harness.default_models());
    harness.pages().refresh_models();
    harness.pick(&drive);

    // The task counts as running until its result is read from the mailbox.
    harness.pages().verify_model(index as i32);
    harness.pages().switch_to_pending_folder();

    assert_eq!(harness.saved().pipeline.models_folder, None);
    assert_eq!(
        harness.seen().pending_folder,
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

    harness.pages().verify_model(index as i32);

    assert!(harness.seen().models_folder_missing);
    assert_eq!(harness.seen().model_error, "");
    assert_eq!(harness.seen().models[index].status, "Folder not connected");
}

#[test]
fn checking_the_folder_again_clears_the_last_error() {
    let harness = Harness::new();
    harness.seen.borrow_mut().model_error = "Couldn't reach the drive".to_owned();

    harness.pages().recheck_models_folder();

    assert_eq!(harness.seen().model_error, "");
}

#[test]
fn picking_the_default_folder_is_the_same_as_use_default() {
    let harness = Harness::new();
    let drive = harness.drive();
    harness.pick(&drive);

    harness.pick(&harness.default_models());

    assert_eq!(harness.saved().pipeline.models_folder, None);
    assert!(!harness.seen().models_folder_custom);
}

#[test]
fn use_default_works_when_the_saved_folder_already_is_the_default() {
    let mut harness = Harness::new();
    let default = harness.default_models();
    SettingsStore::open(harness.settings_path())
        .update(|settings| settings.pipeline.models_folder = Some(default))
        .unwrap();
    harness.restart();
    assert!(harness.seen().models_folder_custom);

    harness.pages().use_default_models_folder();

    assert_eq!(harness.saved().pipeline.models_folder, None);
    assert!(!harness.seen().models_folder_custom);
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

    harness.pages().delete_all();
    fs::set_permissions(locked, fs::Permissions::from_mode(0o755)).unwrap();

    let status = harness.seen().data_status.clone();
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

    harness.pages().open_models_folder();

    assert!(
        harness.seen().models_folder_missing,
        "Check again must appear"
    );
    assert_ne!(harness.seen().model_error, "");

    fs::rename(&unplugged, &drive).unwrap();
    harness.pages().open_models_folder();

    assert!(!harness.seen().models_folder_missing);
    assert_eq!(harness.seen().model_error, "");
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

    harness.pages().delete_all();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

    let status = harness.seen().data_status.clone();
    assert!(status.starts_with("Delete failed"), "{status}");
    assert!(
        status.contains(drive.to_string_lossy().as_ref()),
        "{status}"
    );
}
