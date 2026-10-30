//! Launch at login: the entry that makes the OS start the app, written and removed on request.

#![cfg(not(target_os = "windows"))]

use std::fs;
use std::path::Path;

use wf_platform::autostart::LaunchAtLogin;

#[test]
fn enabling_writes_an_entry_naming_the_app_and_disabling_removes_it() {
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join("agents").join("wispr-free-entry");
    let autostart = LaunchAtLogin::at(&entry, Path::new("/Applications/wispr free/wispr-free"));

    assert!(!autostart.is_enabled());

    autostart.set(true).unwrap();
    assert!(autostart.is_enabled());
    let text = fs::read_to_string(&entry).unwrap();
    assert!(text.contains("wispr free/wispr-free"), "{text}");

    autostart.set(false).unwrap();
    assert!(!autostart.is_enabled());
}

#[test]
fn disabling_twice_is_fine() {
    let dir = tempfile::tempdir().unwrap();
    let autostart = LaunchAtLogin::at(dir.path().join("entry"), Path::new("/bin/app"));

    autostart.set(false).unwrap();
    autostart.set(false).unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn the_launch_agent_escapes_the_path_and_starts_the_app_once_at_login() {
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join("agent.plist");
    LaunchAtLogin::at(&entry, Path::new("/Users/a&b/<app>"))
        .set(true)
        .unwrap();

    let text = fs::read_to_string(&entry).unwrap();
    assert!(
        text.contains("<string>/Users/a&amp;b/&lt;app&gt;</string>"),
        "{text}"
    );
    assert!(text.contains("<key>RunAtLoad</key>"));
    assert!(text.contains("<key>Label</key>"));
    assert!(!text.contains("KeepAlive"), "quitting must stay quit");
}

#[cfg(target_os = "linux")]
#[test]
fn the_desktop_entry_quotes_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join("wispr-free.desktop");
    LaunchAtLogin::at(&entry, Path::new("/opt/wispr free/app"))
        .set(true)
        .unwrap();

    let text = fs::read_to_string(&entry).unwrap();
    assert!(text.contains("Exec=\"/opt/wispr free/app\""), "{text}");
}
