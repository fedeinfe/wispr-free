//! Small asks of the desktop: open a folder or link with the default app, and show or hide the
//! Dock icon.

use std::io;
use std::path::{Path, PathBuf};
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

/// The app bundle `executable` runs from, if any. macOS gives permissions to a bundle: a bare binary
/// can't ask for them.
pub fn bundle_of(executable: &Path) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let inside = macos.file_name()? == "MacOS" && contents.file_name()? == "Contents";
    (inside && bundle.extension()? == "app").then(|| bundle.to_path_buf())
}

/// Opens `bundle` again a moment from now, when this process has quit.
pub fn reopen_after_quit(bundle: &Path) -> io::Result<()> {
    Command::new("/bin/sh")
        .args(["-c", "sleep 1; exec open \"$0\""])
        .arg(bundle)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bundle_is_found_only_from_its_own_binary() {
        let bundle = |path: &str| bundle_of(Path::new(path));
        assert_eq!(
            bundle("/Applications/Wispr Free.app/Contents/MacOS/wispr-free"),
            Some(PathBuf::from("/Applications/Wispr Free.app"))
        );
        assert_eq!(bundle("/repo/target/release/wispr-free"), None);
        assert_eq!(bundle("/repo/Contents/MacOS/wispr-free"), None);
        assert_eq!(bundle("/x.app/Contents/Resources/wispr-free"), None);
        assert_eq!(bundle("wispr-free"), None);
    }
}
