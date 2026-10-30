fn main() {
    // One style everywhere, so the window looks the same on every OS (ADR-0001: own look). Debug
    // info lets the tests find elements in the real UI, and release builds don't need it.
    let debug_build = std::env::var("PROFILE").is_ok_and(|profile| profile == "debug");
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_debug_info(debug_build);
    slint_build::compile_with_config("ui/app.slint", config).expect("compile Slint UI");
}
