//! `settings_probe persist-write` and `persist-check`, for `Shell`: one process changes every
//! setting the way Swift does and exits, and a new process finds every value in the file. Two
//! processes, so nothing can come from memory.
#![cfg(target_os = "macos")]

use std::any::Any;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use tempfile::TempDir;
use wf_audio::InputDevice;
use wf_core::settings::{
    CleanupStrength, HistoryRetention, KeepModels, MediaWhileDictating, PipelineTier, Replacement,
    Settings, SettingsStore, Snippet, Style,
};
use wf_core::shortcut::{Action, Combo, Key, KeyEvent, Side};
use wf_dictation::Dictation;
use wf_models::Catalog;
use wf_platform::keys::KeySignal;
use wf_shell::bridge::{EventSink, PillParts, SaveOutcome, Shell, ShellEvent};
use wf_shell::flow::parts_at;
use wf_shell::paths::Paths;
use wf_shell::settings::values::RuledChange;
use wf_shell::settings::{Desktop, Event, Mailbox};

/// Set by the parent, with the folder to work in.
const FOLDER: &str = "WF_PERSIST_FOLDER";
const MICROPHONES: [&str; 2] = ["usb", "built-in"];

/// Every setting away from its default. Shortcut letters count up in `Action::ALL` order.
fn every_setting_changed() -> Settings {
    let mut settings = Settings::default();
    let general = &mut settings.general;
    general.microphones = MICROPHONES.map(str::to_owned).to_vec();
    general.languages = vec!["en".into(), "it".into()];
    general.launch_at_login = true;
    general.show_pill = false;
    general.show_in_dock = true;
    for (action, letter) in Action::ALL.into_iter().zip('1'..) {
        let keys = [
            Key::Ctrl(Side::Either),
            Key::Alt(Side::Either),
            Key::Char(letter),
        ];
        settings.shortcuts.set(action, Combo::new(keys));
    }
    let pipeline = &mut settings.pipeline;
    pipeline.tier = PipelineTier::Custom;
    pipeline.denoise = true;
    pipeline.vad = false;
    pipeline.llm_cleanup = false;
    pipeline.cleanup_strength = CleanupStrength::High;
    pipeline.vad_auto_stop = true;
    pipeline.vad_auto_stop_seconds = 7;
    pipeline.asr_model = Some("parakeet-tdt-0.6b-v3-int8".into());
    pipeline.vad_model = Some("silero-vad-v5".into());
    pipeline.llm_model = Some("qwen3-1.7b-q4_k_m".into());
    pipeline.keep_models_loaded = KeepModels::Always;
    let style = &mut settings.style;
    style.personal_messages = Style::VeryCasual;
    style.work_messages = Style::Excited;
    style.email = Style::Casual;
    style.other = Style::Excited;
    style.dictionary = vec!["Slint".into(), "Whisper".into()];
    style.replacements = vec![Replacement {
        wrong: "wisper".into(),
        right: "Whisper".into(),
    }];
    style.snippets = vec![Snippet {
        trigger: "my address".into(),
        text: "Via Roma 1".into(),
    }];
    style.auto_add_corrections = false;
    style.press_enter_command = true;
    let meetings = &mut settings.meetings;
    meetings.auto_detect = false;
    meetings.live_transcript = false;
    meetings.refine_pass = false;
    meetings.diarization = false;
    meetings.keep_audio = false;
    meetings.max_length_minutes = 45;
    meetings.summary_template = "Decisions first.".into();
    settings.media.sounds = false;
    settings.media.while_dictating = MediaWhileDictating::Pause;
    settings.privacy.context_awareness = false;
    settings.privacy.history_retention = HistoryRetention::Never;
    settings
}

type KeySink = Arc<Mutex<Option<Box<dyn FnMut(KeySignal) + Send>>>>;
type Reports = Arc<Mutex<Option<Mailbox>>>;

/// Does nothing that reaches the desktop. The test hands the recorder its keys directly.
struct HeadlessDesktop {
    keys: KeySink,
    reports: Reports,
}

impl Desktop for HeadlessDesktop {
    fn bring_to_front(&self) {}

    fn start_probes(&self, mailbox: &Mailbox) {
        *self.reports.lock().unwrap() = Some(mailbox.clone());
    }

    fn set_launch_at_login(&self, _enabled: bool) -> io::Result<()> {
        Ok(())
    }

    fn set_dock_icon_visible(&self, _visible: bool) {}

    fn open(&self, _target: &str) -> io::Result<()> {
        Ok(())
    }

    fn capture_keys(
        &self,
        on_signal: Box<dyn FnMut(KeySignal) + Send>,
    ) -> Result<Box<dyn Any>, String> {
        *self.keys.lock().unwrap() = Some(on_signal);
        Ok(Box::new(()))
    }

    fn test_microphone(
        &self,
        _ranking: Vec<String>,
        _on_level: Box<dyn FnMut(f32) + Send>,
    ) -> Result<Box<dyn Any>, String> {
        Err("not in a headless run".to_owned())
    }

    fn pick_folder(&self, _start: &Path, _done: Box<dyn FnOnce(PathBuf)>) {}
}

struct NoSink;

impl EventSink for NoSink {
    fn on_event(&self, _event: ShellEvent) {}
}

fn now<T>(future: impl Future<Output = T>) -> T {
    use std::task::{Context, Poll, Waker};
    match std::pin::pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("nothing here waits"),
    }
}

fn run_child(folder: &Path, test: &str) {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--ignored", "--nocapture"])
        .env(FOLDER, folder)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{test}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn every_setting_reaches_the_disk_and_a_new_process_reads_it_back() {
    let folder = TempDir::new().unwrap();

    run_child(folder.path(), "child_changes_every_setting");
    run_child(folder.path(), "child_finds_every_setting");
}

/// Plain settings the way `edit` saves them, on the latest revision.
fn edit(shell: &Arc<Shell>, change: impl Fn(&mut Settings)) {
    loop {
        let latest = shell.settings().unwrap();
        let mut settings = latest.settings;
        change(&mut settings);
        let saved = now(Arc::clone(shell).save(settings, latest.revision)).unwrap();
        if matches!(saved, SaveOutcome::Saved { .. }) {
            return;
        }
    }
}

#[test]
#[ignore = "the child of every_setting_reaches_the_disk_and_a_new_process_reads_it_back"]
fn child_changes_every_setting() {
    let Some(folder) = std::env::var_os(FOLDER) else {
        return;
    };
    let wanted = every_setting_changed();
    let paths = Paths::under(Path::new(&folder), "/nowhere/wf-engine-host".into());
    let pill = PillParts {
        prefs: paths.pill.clone(),
        play: Box::new(|_| {}),
        microphones: Box::new(Vec::new),
    };
    let keys = KeySink::default();
    let reports = Reports::default();
    let (for_desktop, reports_for_desktop) = (Arc::clone(&keys), Arc::clone(&reports));
    let store = SettingsStore::open(&paths.settings);
    let shell = Shell::wired_with(
        store,
        Arc::new(NoSink),
        pill,
        paths.clone(),
        Some(Box::new(move || {
            Rc::new(HeadlessDesktop {
                keys: for_desktop,
                reports: reports_for_desktop,
            })
        })),
        None,
        |settings, on_event| Dictation::start(parts_at(&paths, settings, on_event)),
    );
    let settled = || now(shell.pages_state()).unwrap().unwrap();

    // What has a rule, the way the pages' controls ask for it.
    for change in [
        RuledChange::LaunchAtLogin { on: true },
        RuledChange::ShowInDock { on: true },
    ] {
        shell.change_setting(change).unwrap();
    }
    let menus = settled();
    let catalog = Catalog::bundled();
    let option = |menu: &[String], id: &str| {
        let name = &catalog.model(id).unwrap().name;
        let place = menu
            .iter()
            .position(|option| option.ends_with(name.as_str()));
        place.unwrap() as i32
    };
    let pipeline = &wanted.pipeline;
    for change in [
        RuledChange::Tier {
            tier: PipelineTier::Custom,
        },
        RuledChange::AsrModel {
            option: option(
                &menus.asr_menu.options,
                pipeline.asr_model.as_ref().unwrap(),
            ),
        },
        RuledChange::VadModel {
            option: option(
                &menus.vad_menu.options,
                pipeline.vad_model.as_ref().unwrap(),
            ),
        },
        RuledChange::LlmModel {
            option: option(
                &menus.llm_menu.options,
                pipeline.llm_model.as_ref().unwrap(),
            ),
        },
        RuledChange::Denoise { on: true },
        RuledChange::Vad { on: false },
        RuledChange::LlmCleanup { on: false },
    ] {
        shell.change_setting(change).unwrap();
    }
    settled();

    // What the pages save as it is.
    edit(&shell, |settings| {
        let wanted = wanted.clone();
        settings.general.show_pill = wanted.general.show_pill;
        settings.pipeline.cleanup_strength = wanted.pipeline.cleanup_strength;
        settings.pipeline.vad_auto_stop = wanted.pipeline.vad_auto_stop;
        settings.pipeline.vad_auto_stop_seconds = wanted.pipeline.vad_auto_stop_seconds;
        settings.pipeline.keep_models_loaded = wanted.pipeline.keep_models_loaded;
        let style = &mut settings.style;
        style.personal_messages = wanted.style.personal_messages;
        style.work_messages = wanted.style.work_messages;
        style.email = wanted.style.email;
        style.other = wanted.style.other;
        style.auto_add_corrections = wanted.style.auto_add_corrections;
        style.press_enter_command = wanted.style.press_enter_command;
        settings.meetings = wanted.meetings;
        settings.media = wanted.media;
        settings.privacy = wanted.privacy;
    });

    // What goes through a page's own calls.
    for (index, action) in Action::ALL.into_iter().enumerate() {
        shell.record_shortcut(index as i32).unwrap();
        settled();
        let combo = wanted.shortcuts.get(action);
        let press = |event: KeyEvent| {
            let mut keys = keys.lock().unwrap();
            (keys.as_mut().unwrap())(KeySignal::Key(event));
        };
        combo
            .keys()
            .iter()
            .for_each(|&key| press(KeyEvent::press(key)));
        combo
            .keys()
            .iter()
            .for_each(|&key| press(KeyEvent::release(key)));
        settled();
    }

    let devices = MICROPHONES.map(|id| InputDevice {
        id: id.to_owned(),
        name: id.to_owned(),
    });
    now(shell.menu_action(wf_shell::menus::MenuAction::OpenSettings)).unwrap();
    let mailbox = reports.lock().unwrap().clone().unwrap();
    mailbox.post(Event::Microphones {
        devices: devices.to_vec(),
        default: Some(MICROPHONES[0].to_owned()),
    });
    settled();
    shell.set_microphone_auto(false).unwrap();

    shell.set_languages_auto(false).unwrap();
    shell.toggle_language(0).unwrap();
    shell.toggle_language(1).unwrap();
    for word in &wanted.style.dictionary {
        shell.add_word(word.clone()).unwrap();
    }
    for (index, row) in wanted.style.replacements.iter().enumerate() {
        shell.add_replacement().unwrap();
        shell.edit_replacement(index as u32, row.clone()).unwrap();
    }
    for (index, row) in wanted.style.snippets.iter().enumerate() {
        shell.add_snippet().unwrap();
        shell.edit_snippet(index as u32, row.clone()).unwrap();
    }
    settled();
    println!("pid {} set every setting and exits", std::process::id());
}

fn leaves(value: &toml::Value, path: String, found: &mut Vec<(String, toml::Value)>) {
    if let toml::Value::Table(table) = value {
        for (key, value) in table {
            leaves(value, format!("{path}.{key}"), found);
        }
    } else {
        found.push((path, value.clone()));
    }
}

#[test]
#[ignore = "the child of every_setting_reaches_the_disk_and_a_new_process_reads_it_back"]
fn child_finds_every_setting() {
    let Some(folder) = std::env::var_os(FOLDER) else {
        return;
    };
    let read = SettingsStore::open(Path::new(&folder).join("settings.toml")).get();
    let leaves_of = |settings: &Settings| {
        let mut found = Vec::new();
        let value = toml::Value::try_from(settings).unwrap();
        leaves(&value, String::new(), &mut found);
        found
    };
    let wanted = leaves_of(&every_setting_changed());
    let got = leaves_of(&read);
    let defaults = leaves_of(&Settings::default());

    let differing: Vec<_> = wanted.iter().filter(|leaf| !got.contains(leaf)).collect();
    assert_eq!(differing, Vec::<&(String, toml::Value)>::new());
    assert_eq!(got.len(), wanted.len());
    // A value equal to its default would prove nothing. `onboarding.finished` isn't these pages'.
    let unproven: Vec<_> = wanted
        .iter()
        .filter(|leaf| defaults.contains(leaf) && leaf.0 != ".onboarding.finished")
        .collect();
    assert_eq!(unproven, Vec::<&(String, toml::Value)>::new());
}
