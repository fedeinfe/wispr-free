//! The built dev bundle, started the only way a test may start it: its binary directly, with
//! `HOME` on a temp folder, so that it can't reach the user's data and can't take the focus.
//! `cargo xtask bundle --dev` first, then `cargo test -p xtask --test dev_bundle -- --ignored`.
#![cfg(target_os = "macos")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use xtask::bundle::{DEV, target_dir};

const WAIT: Duration = Duration::from_secs(10);

fn binary() -> PathBuf {
    target_dir().join(DEV.app).join("Contents/MacOS/wispr-free")
}

fn start(home: &Path, args: &[&str]) -> Child {
    Command::new(binary())
        .args(args)
        .env("HOME", home)
        .current_dir("/")
        .stderr(Stdio::piped())
        .spawn()
        .expect("`cargo xtask bundle --dev` made the bundle")
}

fn wait_until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        thread::sleep(Duration::from_millis(50));
    }
}

/// Waits for the process to end by itself. One that hangs is killed, and fails the test.
fn wait_for_exit(app: &mut Child) -> Option<i32> {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(status) = app.try_wait().unwrap() {
            return status.code();
        }
        if Instant::now() >= deadline {
            app.kill().unwrap();
            app.wait().unwrap();
            panic!("the app didn't quit");
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn stderr_of(app: &mut Child) -> String {
    let mut text = String::new();
    std::io::Read::read_to_string(app.stderr.as_mut().unwrap(), &mut text).unwrap();
    text
}

#[test]
#[ignore = "needs the built dev bundle"]
fn it_starts_on_its_own_root_and_quits_through_the_shell() {
    let home = TempDir::new().unwrap();
    let support = home.path().join("Library/Application Support");
    let dev = support.join("wispr-free-dev");
    let mut app = start(home.path(), &[]);

    // The flow opens its history once it's up.
    wait_until("the shell starts", || dev.join("history.sqlite3").exists());
    let status = Command::new("kill")
        .arg(app.id().to_string())
        .status()
        .unwrap();
    assert!(status.success());

    assert_eq!(wait_for_exit(&mut app), Some(0));
    assert!(stderr_of(&mut app).contains("shell closed"));
    let roots: Vec<_> = fs::read_dir(&support)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(roots, ["wispr-free-dev"]);
}

#[test]
#[ignore = "needs the built dev bundle"]
fn the_status_menus_quit_goes_through_appkit_and_the_shell() {
    let home = TempDir::new().unwrap();

    let mut app = start(home.path(), &["--rehearse", "--pick=Quit wispr-free"]);

    assert_eq!(wait_for_exit(&mut app), Some(0));
    assert!(stderr_of(&mut app).contains("shell closed"));
}
