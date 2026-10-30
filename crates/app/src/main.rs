slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    if std::env::args()
        .skip(1)
        .any(|arg| arg == "--version" || arg == "-V")
    {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let tray = Tray::new()?;
    tray.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
    tray.show()?;
    slint::run_event_loop_until_quit()
}
