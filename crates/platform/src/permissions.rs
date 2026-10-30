//! What the OS lets this app do (spec §7). The flow asks before it opens the mic, and onboarding
//! can show the same answers. Asking prompts nobody: [`check`] only reads the current state.

#[cfg(target_os = "linux")]
#[path = "permissions/linux.rs"]
mod backend;
#[cfg(target_os = "macos")]
#[path = "permissions/macos.rs"]
mod backend;
#[cfg(target_os = "windows")]
#[path = "permissions/windows.rs"]
mod backend;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Granted,
    /// The user or a policy said no, so trying won't help.
    Denied,
    /// Not decided yet, or the OS won't say. Trying may bring up the OS's own prompt.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permissions {
    pub microphone: Access,
    /// Sending the paste keystroke and reading the focused text.
    pub accessibility: Access,
    /// Seeing shortcuts while another app has focus.
    pub input_monitoring: Access,
}

/// Reads the current state. Fast, but it may load a system framework the first time.
pub fn check() -> Permissions {
    backend::check()
}

impl Permissions {
    /// What to tell the user when the mic can't work, for the pill's tooltip.
    pub fn microphone_problem(&self) -> Option<&'static str> {
        (self.microphone == Access::Denied).then_some(backend::MICROPHONE_DENIED)
    }
}
