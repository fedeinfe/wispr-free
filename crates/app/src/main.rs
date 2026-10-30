slint::include_modules!();

mod debug;
mod mic_levels;
mod pill;
mod prefs;

use pill::PillController;

fn main() -> Result<(), slint::PlatformError> {
    if std::env::args()
        .skip(1)
        .any(|arg| arg == "--version" || arg == "-V")
    {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    pill::select_backend()?;

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
    let events = pill.clone();
    pill.on_event(move |event| debug::handle_event(&events, event));

    if cfg!(debug_assertions) || std::env::args().any(|arg| arg == "--debug-pill") {
        debug::install(&tray, &pill);
    }
    if let Some(state) = debug::state_from_args(std::env::args().skip(1)) {
        pill.set_state(state);
    }

    tray.show()?;
    pill.show()?;
    slint::run_event_loop_until_quit()
}
