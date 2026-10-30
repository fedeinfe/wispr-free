use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use slint::Timer;
use wf_core::settings::{Settings, SettingsStore};
use wf_platform::desktop;
use wispr_free::pill::{self, PillController, PillEvent};
use wispr_free::settings_window::SettingsHost;
use wispr_free::{Tray, debug, prefs};

fn open_settings(settings: &SettingsHost) {
    if let Err(error) = settings.open() {
        eprintln!("could not open settings: {error}");
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

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

    let apply_settings = {
        let pill = pill.clone();
        Rc::new(move |settings: &Settings| {
            pill.set_sounds(settings.media.sounds);
            pill.set_shown(settings.general.show_pill);
        })
    };
    apply_settings(&store.get());
    let settings = SettingsHost::system(Arc::clone(&store), apply_settings);

    tray.on_open_settings({
        let settings = settings.clone();
        move || open_settings(&settings)
    });
    let events = pill.clone();
    pill.on_event({
        let settings = settings.clone();
        move |event| match event {
            PillEvent::OpenSettings => open_settings(&settings),
            event => debug::handle_event(&events, event),
        }
    });

    if cfg!(debug_assertions) || args.iter().any(|arg| arg == "--debug-pill") {
        debug::install(&tray, &pill);
    }
    if let Some(state) = debug::state_from_args(args.iter().cloned()) {
        pill.set_state(state);
    }

    tray.show()?;
    pill.show()?;

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
