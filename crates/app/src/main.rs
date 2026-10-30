use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use slint::Timer;
use wf_core::settings::{Settings, SettingsStore};
use wf_dictation::remote::{ENGINE_HOST_ARG, run_engine_host};
use wf_models::Catalog;
use wf_platform::desktop;
use wispr_free::dictation::{self, PillActions};
use wispr_free::pill::{self, PillController};
use wispr_free::settings_window::SettingsHost;
use wispr_free::{Tray, debug, prefs};

fn open_settings(settings: &SettingsHost) {
    if let Err(error) = settings.open() {
        eprintln!("could not open settings: {error}");
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    // The app runs its engines in copies of itself (ADR-0005). Before anything else, so that a
    // host never touches the window system.
    if args.iter().any(|arg| arg == ENGINE_HOST_ARG) {
        return match run_engine_host() {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        };
    }
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), slint::PlatformError> {
    pill::select_backend()?;

    let store_path =
        SettingsStore::default_path().unwrap_or_else(|| PathBuf::from("settings.toml"));
    let store = Arc::new(SettingsStore::open(store_path));

    let tray = Tray::new()?;
    tray.on_quit(|| {
        let _ = slint::quit_event_loop();
    });

    let prefs_path = prefs::path();
    let prefs = prefs_path
        .as_deref()
        .map(prefs::Prefs::load)
        .unwrap_or_default();
    let pill = PillController::new(prefs, prefs_path)?;
    let clip_run = debug::ClipRun::from_args(args);
    let flow = dictation::start(store.get(), &pill, &tray, clip_run.as_ref());
    tray.on_toggle_meeting({
        let flow = flow.clone();
        move || flow.toggle_meeting()
    });

    let apply_settings: Rc<dyn Fn(&Settings)> = {
        let pill = pill.clone();
        let flow = flow.clone();
        Rc::new(move |settings: &Settings| {
            pill.set_sounds(settings.media.sounds);
            pill.set_shown(settings.general.show_pill);
            flow.settings_changed(settings.clone());
        })
    };
    apply_settings(&store.get());
    let settings =
        SettingsHost::system(Arc::clone(&store), Rc::clone(&apply_settings), flow.clone());

    tray.on_open_settings({
        let settings = settings.clone();
        move || open_settings(&settings)
    });
    let actions = PillActions {
        dictation: flow.clone(),
        pill: pill.clone(),
        store: Arc::clone(&store),
        settings: settings.clone(),
        catalog: Arc::new(Catalog::bundled()),
        apply: apply_settings,
    };
    pill.on_event(move |event| actions.handle(event));

    if cfg!(debug_assertions) || args.iter().any(|arg| arg == "--debug-pill") {
        debug::install(&tray, &pill);
    }
    if let Some(state) = debug::state_from_args(args.iter().cloned()) {
        pill.set_state(state);
    }

    tray.show()?;
    pill.show()?;
    let _keys = match clip_run {
        Some(clip_run) => {
            clip_run.drive(flow.clone());
            None
        }
        None => Some(dictation::listen_to_keys(&flow)),
    };

    // A Dock icon needs the running event loop. The loop starts without one, so that launching
    // never takes the foreground from the app in use.
    let show_in_dock = store.get().general.show_in_dock;
    let open_at_launch = args.iter().any(|arg| arg == "--settings");
    Timer::single_shot(Duration::ZERO, move || {
        if show_in_dock {
            desktop::set_dock_icon_visible(true);
        }
        if open_at_launch {
            open_settings(&settings);
        }
    });
    slint::run_event_loop_until_quit()
}
