//! The app: its windows and the wiring between them and the other crates.

pub mod debug;
pub mod dictation;
mod folder_picker;
pub mod meetings_window;
mod mic_levels;
pub mod onboarding;
pub mod pill;
pub mod settings_window;
mod slint_clock;

pub use folder_picker::folder_picker;
pub use wf_shell::{memory, prefs};

slint::include_modules!();
