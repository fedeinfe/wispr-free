//! The app's decisions, free of any window toolkit: what the windows show and what their controls
//! do. A front end draws the result and reports what the user does.

pub mod app_windows;
pub mod bridge;
pub mod clock;
pub mod flow;
pub mod lists;
pub mod meetings;
pub mod memory;
pub mod menus;
pub mod mic_levels;
pub mod onboarding;
pub mod paths;
pub mod pill;
pub mod prefs;
pub mod rehearsal;
pub mod settings;
pub mod windows;

uniffi::setup_scaffolding!("wfshell");
