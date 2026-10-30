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
    macos::set_activation_policy(if visible {
        macos::POLICY_REGULAR
    } else {
        macos::POLICY_ACCESSORY
    });
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
    use std::ffi::{CStr, c_void};

    pub const POLICY_REGULAR: isize = 0;
    pub const POLICY_ACCESSORY: isize = 1;

    type Object = *mut c_void;
    type Selector = *const c_void;

    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {}

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const std::ffi::c_char) -> Object;
        fn sel_registerName(name: *const std::ffi::c_char) -> Selector;
        fn objc_msgSend();
    }

    fn selector(name: &CStr) -> Selector {
        unsafe { sel_registerName(name.as_ptr()) }
    }

    fn shared_application() -> Object {
        let send: extern "C" fn(Object, Selector) -> Object =
            unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        let class = unsafe { objc_getClass(c"NSApplication".as_ptr()) };
        send(class, selector(c"sharedApplication"))
    }

    pub fn set_activation_policy(policy: isize) {
        let send: extern "C" fn(Object, Selector, isize) -> bool =
            unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        send(
            shared_application(),
            selector(c"setActivationPolicy:"),
            policy,
        );
    }

    pub fn activate() {
        let send: extern "C" fn(Object, Selector, bool) =
            unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        send(
            shared_application(),
            selector(c"activateIgnoringOtherApps:"),
            true,
        );
    }
}
