//! Small asks of the desktop: open a folder or link with the default app, and show or hide the
//! Dock icon.

use std::io;
use std::process::{Command, Stdio};

/// Opens a folder or URL with the OS default handler.
pub fn open(target: &str) -> io::Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(target_os = "windows") {
        Command::new("explorer")
    } else {
        Command::new("xdg-open")
    };
    command
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
}

/// Shows or hides the app's Dock icon (macOS). Call on the main thread, with the event loop
/// running. Elsewhere there's nothing to do.
pub fn set_dock_icon_visible(visible: bool) {
    #[cfg(target_os = "macos")]
    macos::set_dock_icon_visible(visible);
    #[cfg(not(target_os = "macos"))]
    let _ = visible;
}

/// Brings the app to the front, which an app without a Dock icon doesn't get by opening a window.
/// Call on the main thread.
pub fn activate_app() {
    #[cfg(target_os = "macos")]
    macos::activate();
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    pub fn set_dock_icon_visible(visible: bool) {
        let Some(main_thread) = MainThreadMarker::new() else {
            return;
        };
        NSApplication::sharedApplication(main_thread).setActivationPolicy(if visible {
            NSApplicationActivationPolicy::Regular
        } else {
            NSApplicationActivationPolicy::Accessory
        });
    }

    pub fn activate() {
        if let Some(main_thread) = MainThreadMarker::new() {
            // Deprecated, but its replacement needs macOS 14 and the app supports 13.
            #[allow(deprecated)]
            NSApplication::sharedApplication(main_thread).activateIgnoringOtherApps(true);
        }
    }
}
