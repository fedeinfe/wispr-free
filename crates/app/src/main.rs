use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use slint::Timer;
use wf_core::settings::{Settings, SettingsStore};
use wf_dictation::remote::{ENGINE_HOST_ARG, run_engine_host};
use wf_models::Catalog;
use wf_platform::autostart::LaunchAtLogin;
use wf_platform::desktop;
use wispr_free::dictation::{self, KeyWatch, PillActions};
use wispr_free::onboarding::{
    FlowWatcher, OnboardingHost, OnboardingParts, Opening, SystemMachine,
};
use wispr_free::pill::{self, PillController};
use wispr_free::settings_window::{SettingsHost, system_parts};
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
    let watcher = FlowWatcher::default();
    let flow = dictation::start(
        store.get(),
        &pill,
        &tray,
        clip_run.as_ref(),
        Rc::clone(&watcher),
    );
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
        apply: Rc::clone(&apply_settings),
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
    let bundle = env::current_exe()
        .ok()
        .and_then(|executable| desktop::bundle_of(&executable));
    let mut onboarding = None;
    let _keys = match (clip_run, bundle) {
        (Some(clip_run), _) => {
            clip_run.drive(flow.clone());
            None
        }
        // Permissions belong to a bundle, so a bare binary has nothing to walk anyone through.
        (None, None) => Some(dictation::listen_to_keys(&flow)),
        (None, Some(bundle)) => {
            let keys = KeyWatch::idle(&flow);
            let machine = SystemMachine {
                bundle,
                dictation: flow.clone(),
                keys: keys.clone(),
                settings: settings.clone(),
                tray: tray.as_weak(),
            };
            let host = OnboardingHost::new(OnboardingParts {
                settings: system_parts(
                    Arc::clone(&store),
                    Rc::clone(&apply_settings),
                    flow.clone(),
                ),
                machine: Rc::new(machine),
            });
            watcher.replace(Some(Box::new({
                let host = host.clone();
                move |event| host.on_flow_event(event)
            })));
            settings.redirect_with({
                let host = host.clone();
                move || host.bring_forward()
            });
            tray.on_open_setup({
                let host = host.clone();
                move || {
                    if let Err(error) = host.open(Opening::Foreground) {
                        eprintln!("could not open onboarding: {error}");
                    }
                }
            });
            onboarding = Some(host);
            Some(keys)
        }
    };

    // A Dock icon needs the running event loop. The loop starts without one, so that launching
    // never takes the foreground from the app in use.
    let show_in_dock = store.get().general.show_in_dock;
    let open_at_launch = args.iter().any(|arg| arg == "--settings");
    let login_entry = store.get().general.launch_at_login && onboarding.is_some();
    Timer::single_shot(Duration::ZERO, move || {
        if show_in_dock {
            desktop::set_dock_icon_visible(true);
        }
        // The app may have moved since the entry was written.
        if login_entry && let Err(error) = LaunchAtLogin::system().and_then(|entry| entry.set(true))
        {
            eprintln!("could not update launch at login: {error}");
        }
        if let Some(onboarding) = &onboarding
            && let Err(error) = onboarding.start()
        {
            eprintln!("could not open onboarding: {error}");
        }
        if open_at_launch {
            open_settings(&settings);
        }
    });
    slint::run_event_loop_until_quit()
}
