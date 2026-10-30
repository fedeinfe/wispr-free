//! Headless checks of the Settings window, safe on a machine in use: Slint's software renderer
//! draws into memory, so there is no OS window, no focus change and no key or mic capture, and
//! all files live in a temporary folder.
//!
//!   cargo run --release -p wispr-free --example settings_probe -- memory [cycles]
//!   cargo run --release -p wispr-free --example settings_probe -- render <folder>
//!   cargo run --release -p wispr-free --example settings_probe -- persist-write <folder>
//!   cargo run --release -p wispr-free --example settings_probe -- persist-check <folder>
//!
//! `memory` opens and closes the window through `SettingsHost`, drawing every page each time, and
//! prints `phys_footprint` in MB (macOS). It covers what the app itself holds. The running app
//! draws with femtovg, so its GL memory, AppKit and the window's surface are not in these numbers:
//! they need a real window, which only a person should open on a machine in use.
//!
//! `persist-write` changes every setting through the window and exits. `persist-check` is a new
//! process: it compares every value in the file with what was set and fails on any difference.

use std::any::Any;
use std::cell::Cell;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use i_slint_backend_testing::{TestingBackend, TestingBackendOptions};
use slint::{ComponentHandle, LogicalSize, Timer, TimerMode};
use wf_core::settings::{
    CleanupStrength, HistoryRetention, KeepModels, MediaWhileDictating, PipelineTier, Replacement,
    Settings, SettingsStore, Snippet, Style, Theme,
};
use wf_core::shortcut::{Action, Combo, Key, KeyEvent, Platform, Side};
use wf_models::Catalog;
use wf_platform::keys::KeySignal;
use wispr_free::settings_window::values::{ModelChoices, values_of};
use wispr_free::settings_window::{
    Desktop, Event, Mailbox, Paths, SettingsController, SettingsHost, SettingsParts, system,
};
use wispr_free::{Prefs, ReplacementRow, SettingsWindow, SnippetRow};

const PAGES: i32 = 8;
const PAGE_NAMES: [&str; PAGES as usize] = [
    "general", "models", "pipeline", "style", "meetings", "sounds", "privacy", "about",
];

/// Does nothing that reaches the desktop, except for the read-only hardware and device probes.
struct HeadlessDesktop;

impl Desktop for HeadlessDesktop {
    fn bring_to_front(&self) {}

    fn start_probes(&self, mailbox: &Mailbox) {
        system::start_probes(mailbox);
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
        _on_signal: Box<dyn FnMut(KeySignal) + Send>,
    ) -> Result<Box<dyn Any>, String> {
        // Nothing is captured: `persist-write` hands the recorder its keys directly.
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

fn parts(folder: &Path) -> SettingsParts {
    SettingsParts {
        store: Arc::new(SettingsStore::open(folder.join("settings.toml"))),
        catalog: Arc::new(Catalog::bundled()),
        desktop: Rc::new(HeadlessDesktop),
        on_change: Rc::new(|_| {}),
        paths: Paths {
            data_dir: folder.join("data"),
            export_dir: folder.join("exports"),
        },
        platform: Platform::current(),
    }
}

fn draw_every_page(window: &SettingsWindow) {
    for page in 0..PAGES {
        window.set_page(page);
        window.window().take_snapshot().expect("page renders");
    }
}

#[cfg(target_os = "macos")]
fn footprint_mb() -> f64 {
    #[repr(C)]
    #[derive(Default)]
    struct RusageInfoV0 {
        uuid: [u8; 16],
        user_time: u64,
        system_time: u64,
        pkg_idle_wkups: u64,
        interrupt_wkups: u64,
        pageins: u64,
        wired_size: u64,
        resident_size: u64,
        phys_footprint: u64,
        proc_start_abstime: u64,
        proc_exit_abstime: u64,
    }
    unsafe extern "C" {
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut RusageInfoV0) -> i32;
    }
    let mut info = RusageInfoV0::default();
    unsafe { proc_pid_rusage(std::process::id() as i32, 0, &mut info) };
    info.phys_footprint as f64 / (1024.0 * 1024.0)
}

#[cfg(not(target_os = "macos"))]
fn footprint_mb() -> f64 {
    0.0
}

fn measure(cycles: u32) {
    let folder = tempfile::tempdir().expect("temporary folder");
    let parts = parts(folder.path());
    let host = SettingsHost::new(Rc::new(move || parts.clone()));
    println!("start,{:.1}", footprint_mb());
    println!("cycle,open_mb,closed_before_collect_mb,closed_mb");

    let step = Cell::new(0u32);
    let (open, closing) = (Cell::new(0.0), Cell::new(0.0));
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(100), move || {
        let tick = step.get();
        step.set(tick + 1);
        // 1 s open, then 2 s closed: longer than the teardown delay before the collect.
        match tick % 30 {
            0 if tick / 30 >= cycles => {
                let _ = slint::quit_event_loop();
            }
            0 => host.open().expect("window opens"),
            9 => {
                let window = host.window().and_then(|window| window.upgrade());
                draw_every_page(&window.expect("window is open"));
                open.set(footprint_mb());
                host.close();
            }
            12 => closing.set(footprint_mb()),
            29 => println!(
                "{},{:.1},{:.1},{:.1}",
                tick / 30 + 1,
                open.get(),
                closing.get(),
                footprint_mb()
            ),
            _ => {}
        }
    });
    slint::run_event_loop_until_quit().expect("event loop runs");
}

fn save_png(window: &SettingsWindow, path: &Path) {
    let snapshot = window.window().take_snapshot().expect("page renders");
    image::save_buffer(
        path,
        snapshot.as_bytes(),
        snapshot.width(),
        snapshot.height(),
        image::ColorType::Rgba8,
    )
    .expect("png is written");
}

/// So the pages show a downloaded model next to missing ones.
fn fake_a_downloaded_model(parts: &SettingsParts) {
    let catalog = Catalog::bundled();
    let model = catalog.model("silero-vad-v5").expect("in the catalog");
    let folder = parts.paths.data_dir.join("models").join(&model.id);
    std::fs::create_dir_all(&folder).expect("model folder");
    for file in &model.files {
        std::fs::write(folder.join(&file.name), vec![0; file.size as usize]).expect("model file");
    }
}

fn render(target: &Path) {
    std::fs::create_dir_all(target).expect("output folder");
    let folder = tempfile::tempdir().expect("temporary folder");
    let parts = parts(folder.path());
    fake_a_downloaded_model(&parts);
    parts
        .store
        .update(|settings| {
            settings.style.dictionary = vec!["Slint".into(), "Whisper".into()];
            settings.style.replacements = vec![Replacement {
                wrong: "wisper".into(),
                right: "Whisper".into(),
            }];
            settings.style.snippets = vec![Snippet {
                trigger: "my address".into(),
                text: "Via Roma 1, Milano".into(),
            }];
        })
        .expect("sample settings");

    for (theme, name) in [(Theme::Light, "light"), (Theme::Dark, "dark")] {
        parts
            .store
            .update(|settings| settings.general.theme = theme)
            .expect("theme");
        let controller = SettingsController::new(parts.clone()).expect("window builds");
        controller.handle(Event::Hardware {
            summary: "10 CPU cores · 16.0 GB RAM · Sample GPU (Metal)".to_owned(),
            recommended: "Balanced".to_owned(),
        });
        let window = controller.window();
        window.window().set_size(LogicalSize::new(880.0, 640.0));
        window.global::<Prefs>().set_languages_auto(false);
        window.global::<Prefs>().set_microphone_auto(false);
        for page in 0..PAGES {
            window.set_page(page);
            let file = format!("{name}-{}.png", PAGE_NAMES[page as usize]);
            save_png(window, &target.join(file));
        }
        window.set_page(0);
        window.window().set_size(LogicalSize::new(880.0, 1500.0));
        save_png(window, &target.join(format!("{name}-general-full.png")));
        window.set_page(1);
        save_png(window, &target.join(format!("{name}-models-full.png")));
        window.set_page(2);
        save_png(window, &target.join(format!("{name}-pipeline-full.png")));
    }
}

const MICROPHONES: [&str; 2] = ["usb", "built-in"];

/// Every setting away from its default. Shortcut letters count up in `Action::ALL` order.
fn every_setting_changed() -> Settings {
    let mut settings = Settings::default();
    let general = &mut settings.general;
    general.microphones = MICROPHONES.map(str::to_owned).to_vec();
    general.languages = vec!["en".into(), "it".into()];
    general.app_language = "en".into();
    general.theme = Theme::Dark;
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
    pipeline.asr_model = Some("whisper-small-q8_0".into());
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
    let meetings = &mut settings.meetings;
    meetings.auto_detect = true;
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

fn persist_write(folder: &Path) {
    let parts = parts(folder);
    let (catalog, wanted) = (Arc::clone(&parts.catalog), every_setting_changed());
    let controller = SettingsController::new(parts).expect("window builds");
    let window = controller.window();
    let prefs = window.global::<Prefs>();

    let choices = ModelChoices::from_catalog(&catalog);
    prefs.set_values(values_of(&wanted, &catalog, &choices));
    prefs.invoke_commit();

    for (index, action) in Action::ALL.into_iter().enumerate() {
        prefs.invoke_record_shortcut(index as i32);
        let keys = wanted.shortcuts.get(action).keys();
        for &key in keys {
            controller.handle(Event::Key(KeySignal::Key(KeyEvent::press(key))));
        }
        for &key in keys {
            controller.handle(Event::Key(KeySignal::Key(KeyEvent::release(key))));
        }
    }

    let devices = MICROPHONES.map(|id| wf_audio::InputDevice {
        id: id.to_owned(),
        name: id.to_owned(),
    });
    controller.handle(Event::Microphones {
        devices: devices.to_vec(),
        default: Some(MICROPHONES[0].to_owned()),
    });
    prefs.invoke_set_microphone_auto(false);

    prefs.invoke_set_languages_auto(false);
    prefs.invoke_toggle_language(0);
    prefs.invoke_toggle_language(1);
    for word in &wanted.style.dictionary {
        prefs.invoke_add_word(word.as_str().into());
    }
    for (index, replacement) in wanted.style.replacements.iter().enumerate() {
        prefs.invoke_add_replacement();
        let row = ReplacementRow {
            wrong: replacement.wrong.as_str().into(),
            right: replacement.right.as_str().into(),
        };
        prefs.invoke_edit_replacement(index as i32, row);
    }
    for (index, snippet) in wanted.style.snippets.iter().enumerate() {
        prefs.invoke_add_snippet();
        let row = SnippetRow {
            trigger: snippet.trigger.as_str().into(),
            text: snippet.text.as_str().into(),
        };
        prefs.invoke_edit_snippet(index as i32, row);
    }
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

fn persist_check(folder: &Path) -> bool {
    let read = SettingsStore::open(folder.join("settings.toml")).get();
    let leaves_of = |settings: &Settings| {
        let mut found = Vec::new();
        let value = toml::Value::try_from(settings).expect("settings serialize");
        leaves(&value, String::new(), &mut found);
        found
    };
    let wanted = leaves_of(&every_setting_changed());
    let got = leaves_of(&read);
    let defaults = leaves_of(&Settings::default());
    let differing: Vec<_> = wanted.iter().filter(|leaf| !got.contains(leaf)).collect();
    let still_default = wanted.iter().filter(|leaf| defaults.contains(leaf)).count();
    println!(
        "pid {} compared {} values: {} differ, {} of them equal to a default",
        std::process::id(),
        wanted.len(),
        differing.len(),
        still_default,
    );
    for (path, value) in &differing {
        println!("  {path}: expected {value}");
    }
    differing.is_empty() && still_default == 0 && got.len() == wanted.len()
}

fn main() -> ExitCode {
    let backend = TestingBackend::new(TestingBackendOptions {
        mock_time: false,
        threading: true,
        renderer_name: Some("software".into()),
    });
    slint::platform::set_platform(Box::new(backend)).expect("headless platform");

    let mut args = std::env::args().skip(1);
    match (args.next().as_deref(), args.next()) {
        (Some("memory"), cycles) => measure(cycles.map_or(10, |n| n.parse().expect("a number"))),
        (Some("render"), Some(folder)) => render(Path::new(&folder)),
        (Some("persist-write"), Some(folder)) => persist_write(Path::new(&folder)),
        (Some("persist-check"), Some(folder)) => {
            if !persist_check(Path::new(&folder)) {
                return ExitCode::FAILURE;
            }
        }
        _ => {
            eprintln!(
                "usage: settings_probe memory [cycles] | render|persist-write|persist-check <folder>"
            );
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
