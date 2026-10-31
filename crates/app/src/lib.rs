//! The app: its windows and the wiring between them and the other crates.

pub mod debug;
pub mod dictation;
pub mod meetings_window;
mod mic_levels;
pub mod onboarding;
pub mod pill;
pub mod settings_window;

pub use wf_shell::{memory, prefs};

slint::include_modules!();
