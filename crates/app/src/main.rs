use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use slint::Timer;
use wf_core::settings::SettingsStore;
use wf_platform::desktop;
use wispr_free::settings_window::SettingsHost;
use wispr_free::{Tray, select_renderer};

fn main() -> Result<(), slint::PlatformError> {
    let mut args = std::env::args().skip(1);
    let open_settings = args.any(|arg| arg == "--settings");
    if std::env::args()
        .skip(1)
        .any(|arg| arg == "--version" || arg == "-V")
    {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    select_renderer()?;
    let path = SettingsStore::default_path().unwrap_or_else(|| PathBuf::from("settings.toml"));
    let store = Arc::new(SettingsStore::open(path));
    let settings = SettingsHost::system(Arc::clone(&store));

    let tray = Tray::new()?;
    tray.on_open_settings({
        let settings = settings.clone();
        move || {
            if let Err(error) = settings.open() {
                eprintln!("could not open settings: {error}");
            }
        }
    });
    tray.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
    tray.show()?;

    // The Dock icon is the event loop's to set at launch, so this has to run inside it.
    let show_in_dock = store.get().general.show_in_dock;
    Timer::single_shot(Duration::ZERO, move || {
        desktop::set_dock_icon_visible(show_in_dock);
        if open_settings && let Err(error) = settings.open() {
            eprintln!("could not open settings: {error}");
        }
    });
    slint::run_event_loop_until_quit()
}
