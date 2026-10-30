//! The app: its windows and the wiring between them and the other crates.

pub mod debug;
pub mod dictation;
pub mod meetings_window;
pub mod memory;
mod mic_levels;
pub mod onboarding;
pub mod pill;
pub mod prefs;
pub mod settings_window;

slint::include_modules!();
