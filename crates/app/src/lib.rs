//! The app: its windows and the wiring between them and the other crates.

pub mod debug;
pub mod memory;
mod mic_levels;
pub mod pill;
pub mod prefs;
pub mod settings_window;

slint::include_modules!();
