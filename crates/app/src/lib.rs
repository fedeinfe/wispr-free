//! The app: its windows and the wiring between them and the other crates.

pub mod memory;
pub mod settings_window;

use slint::PlatformError;

slint::include_modules!();

/// Picks the software renderer unless `SLINT_BACKEND` says otherwise. On macOS an OpenGL window of
/// any size costs about 200 MB that never comes back, against the 40 MB idle budget of spec goal 4
/// (ADR-0002). Call before creating any window.
pub fn select_renderer() -> Result<(), PlatformError> {
    if std::env::var_os("SLINT_BACKEND").is_some() {
        return Ok(());
    }
    slint::BackendSelector::new()
        .renderer_name("software".to_owned())
        .select()
}
