//! What the OS lets this app do (spec §7). The flow checks before it opens the mic, and onboarding
//! shows the same answers. [`check`] only reads the current state and prompts nobody; only [`ask`]
//! ever brings up a question from the OS.

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

/// One thing the user has a say on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Microphone,
    Accessibility,
    InputMonitoring,
}

/// Reads the current state. Fast, but it may load a system framework the first time.
pub fn check() -> Permissions {
    backend::check()
}

/// Brings up the OS's own question and returns at once, so the answer shows in a later [`check`].
/// The OS asks only once. After that nothing shows, and the switch is at [`settings_link`].
///
/// macOS needs the app's bundle for this: it ends a process that asks for the mic with no usage
/// description to show.
pub fn ask(permission: Permission) {
    backend::ask(permission);
}

/// The page of the OS's settings that holds `permission`, for [`crate::desktop::open`].
pub fn settings_link(permission: Permission) -> Option<&'static str> {
    backend::settings_link(permission)
}

impl Permissions {
    pub fn get(&self, permission: Permission) -> Access {
        match permission {
            Permission::Microphone => self.microphone,
            Permission::Accessibility => self.accessibility,
            Permission::InputMonitoring => self.input_monitoring,
        }
    }

    /// What to tell the user when the mic can't work, for the pill's tooltip.
    pub fn microphone_problem(&self) -> Option<&'static str> {
        (self.microphone == Access::Denied).then_some(backend::MICROPHONE_DENIED)
    }
}
