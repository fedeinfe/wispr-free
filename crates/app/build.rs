fn main() {
    // One style everywhere, so the window looks the same on every OS (ADR-0001: own look). Debug
    // builds keep element ids so the UI tests can find elements.
    let debug = std::env::var("PROFILE").is_ok_and(|profile| profile == "debug");
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_debug_info(debug);
    slint_build::compile_with_config("ui/app.slint", config).expect("compile Slint UI");
    // cpal imports macOS 14.2's process tap, which would stop the app from starting on 13 before
    // the ScreenCaptureKit fallback gets a chance (ticket 15).
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "macos") {
        println!("cargo:rustc-link-arg-bins=-Wl,-weak_framework,CoreAudio");
    }
}
