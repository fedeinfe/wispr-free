//! The dev app must never reach the user's data, however it gets started (ticket 09).
//!
//! `ProjectDirs` finds the real root through `HOME`, so these run `Shell` in a child process whose
//! `HOME` is a temp folder. Anything that falls back to the real root lands in that folder's
//! `wispr-free`, seeded here with files that would show it, and the user's own stays out of reach
//! even when the code is wrong.
#![cfg(target_os = "macos")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use tempfile::TempDir;
use wf_core::settings::Settings;
use wf_shell::bridge::{EventSink, SaveOutcome, Shell, ShellError, ShellEvent};
use wf_shell::flow::parts_at;
use wf_shell::paths::{Paths, dev_paths};

type AimAtReal = fn(&mut Paths, PathBuf);

/// Set by the parent, so the child tests do nothing when `--ignored` runs them directly.
const CHILD: &str = "WF_ISOLATION_CHILD";
const SUPPORT: &str = "Library/Application Support";

fn real_root(home: &Path) -> PathBuf {
    home.join(SUPPORT).join("wispr-free")
}

fn dev_root(home: &Path) -> PathBuf {
    home.join(SUPPORT).join("wispr-free-dev")
}

/// What the user's real folder holds, each with something a reader or a writer would show up in.
fn seed_real_root(home: &Path) {
    let root = real_root(home);
    fs::create_dir_all(root.join("meetings/2026-01-01-canary")).unwrap();
    fs::create_dir_all(root.join("models")).unwrap();
    fs::write(root.join("settings.toml"), "[general]\nshow_pill = false\n").unwrap();
    fs::write(root.join("pill.toml"), "edge = \"left\"\nsounds = false\n").unwrap();
    fs::write(root.join("history.sqlite3"), "canary").unwrap();
    fs::write(root.join("media-note.json"), "canary").unwrap();
    fs::write(
        root.join("meetings/2026-01-01-canary/meeting.json"),
        "canary",
    )
    .unwrap();
    fs::write(root.join("models/canary.gguf"), "canary").unwrap();
}

/// Every file below `folder`, with what it holds and when it last changed.
fn snapshot(folder: &Path) -> Vec<(PathBuf, Vec<u8>, SystemTime)> {
    let mut files: Vec<_> = walk(folder)
        .into_iter()
        .map(|path| {
            let changed = fs::metadata(&path).unwrap().modified().unwrap();
            let bytes = fs::read(&path).unwrap();
            (path, bytes, changed)
        })
        .collect();
    files.sort();
    files
}

fn walk(folder: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = fs::read_dir(folder) else {
        return files;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else {
            files.push(path);
        }
    }
    files
}

/// Runs one `#[ignore]`d test of this file in a process with `home` as its `HOME`.
fn run_child(home: &Path, test: &str) {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--ignored", "--nocapture"])
        .env("HOME", home)
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{test}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn in_child() -> bool {
    std::env::var_os(CHILD).is_some()
}

struct NoSink;

impl EventSink for NoSink {
    fn on_event(&self, _event: ShellEvent) {}
}

/// Says when the flow's insertion thread has got through what it does at startup.
struct Toasts(Mutex<Sender<String>>);

impl EventSink for Toasts {
    fn on_event(&self, event: ShellEvent) {
        if let ShellEvent::Toast { text } = event {
            let _ = self.0.lock().unwrap().send(text);
        }
    }
}

/// Runs a call of `Shell` that has nothing to wait for.
fn now<T>(future: impl Future<Output = T>) -> T {
    use std::task::{Context, Poll, Waker};
    match std::pin::pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("nothing here waits"),
    }
}

#[test]
fn a_dev_start_leaves_the_real_root_alone() {
    let home = TempDir::new().unwrap();
    seed_real_root(home.path());
    let before = snapshot(&real_root(home.path()));

    run_child(home.path(), "child_starts_saves_and_quits");

    assert_eq!(snapshot(&real_root(home.path())), before);
    let settings = fs::read_to_string(dev_root(home.path()).join("settings.toml")).unwrap();
    assert!(settings.contains("dev-only-word"));
    assert!(dev_root(home.path()).join("history.sqlite3").is_file());
    let elsewhere: Vec<_> = walk(home.path())
        .into_iter()
        .filter(|path| {
            !path.starts_with(real_root(home.path())) && !path.starts_with(dev_root(home.path()))
        })
        .collect();
    assert_eq!(elsewhere, Vec::<PathBuf>::new());
}

#[test]
#[ignore = "the child of a_dev_start_leaves_the_real_root_alone"]
fn child_starts_saves_and_quits() {
    if !in_child() {
        return;
    }
    let (toasts, toast) = mpsc::channel();
    let sink = Arc::new(Toasts(Mutex::new(toasts)));
    let shell = now(Shell::start(dev_paths().unwrap(), sink)).unwrap();

    let current = shell.settings().unwrap();
    assert!(current.settings.general.show_pill, "read the real settings");
    let mut changed = current.settings.clone();
    changed.style.dictionary.push("dev-only-word".to_owned());
    let saved = now(Arc::clone(&shell).save(changed, current.revision)).unwrap();
    assert!(matches!(saved, SaveOutcome::Saved { .. }));
    shell.paste_last().unwrap();
    toast.recv_timeout(Duration::from_secs(10)).unwrap();
    now(shell.quit()).unwrap();
}

#[test]
fn a_start_on_the_real_root_is_refused_and_touches_nothing() {
    let home = TempDir::new().unwrap();
    seed_real_root(home.path());
    let before = snapshot(&real_root(home.path()));

    run_child(home.path(), "child_is_refused_every_real_path");

    assert_eq!(snapshot(&real_root(home.path())), before);
}

#[test]
#[ignore = "the child of a_start_on_the_real_root_is_refused_and_touches_nothing"]
fn child_is_refused_every_real_path() {
    if !in_child() {
        return;
    }
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let real = real_root(&home);
    let safe = dev_paths().unwrap();
    let cases: [(&str, AimAtReal); 5] = [
        ("settings", |paths, real| {
            paths.settings = real.join("settings.toml");
        }),
        ("pill", |paths, real| paths.pill = real.join("pill.toml")),
        ("models", |paths, real| paths.models = real.join("models")),
        ("history", |paths, real| {
            paths.history = real.join("history.sqlite3");
        }),
        ("meetings", |paths, real| {
            paths.meetings = real.join("meetings");
        }),
    ];
    for (name, aim_at_real) in cases {
        let mut paths = safe.clone();
        aim_at_real(&mut paths, real.clone());

        let started = now(Shell::start(paths, Arc::new(NoSink)));

        assert!(
            matches!(started, Err(ShellError::RealData { .. })),
            "{name}"
        );
    }
}

#[test]
fn a_relative_or_climbing_path_is_refused_rather_than_resolved() {
    let home = TempDir::new().unwrap();

    run_child(home.path(), "child_is_refused_unclear_paths");

    assert_eq!(walk(home.path()), Vec::<PathBuf>::new());
}

#[test]
#[ignore = "the child of a_relative_or_climbing_path_is_refused_rather_than_resolved"]
fn child_is_refused_unclear_paths() {
    if !in_child() {
        return;
    }
    let safe = dev_paths().unwrap();
    let climbing = safe
        .settings
        .parent()
        .unwrap()
        .join("../wispr-free/settings.toml");
    for path in [PathBuf::new(), "settings.toml".into(), climbing] {
        let paths = Paths {
            settings: path.clone(),
            ..safe.clone()
        };

        let started = now(Shell::start(paths, Arc::new(NoSink)));

        assert!(
            matches!(started, Err(ShellError::UnclearPath { .. })),
            "{path:?}"
        );
    }
}

#[test]
fn every_dev_path_has_its_own_place_under_the_dev_root() {
    let root = Path::new("/Users/someone/Library/Application Support/wispr-free-dev");

    let paths = Paths::under(root, "/app/Contents/Helpers/wf-engine-host".into());

    assert_eq!(
        paths,
        Paths {
            settings: root.join("settings.toml"),
            pill: root.join("pill.toml"),
            models: root.join("models"),
            history: root.join("history.sqlite3"),
            meetings: root.join("meetings"),
            engine_host: "/app/Contents/Helpers/wf-engine-host".into(),
        }
    );
}

/// The flow reads some of these only when there's something to read, which a run can't show.
#[test]
fn the_flow_gets_every_file_from_the_given_paths() {
    let root = Path::new("/Users/someone/Library/Application Support/wispr-free-dev");
    let paths = Paths::under(root, "/app/wf-engine-host".into());

    let parts = parts_at(&paths, Settings::default(), Arc::new(|_| {}));

    assert_eq!(parts.default_models, root.join("models"));
    assert_eq!(parts.history, root.join("history.sqlite3"));
    assert_eq!(parts.media_note, root.join("media-note.json"));
    assert_eq!(parts.meetings, root.join("meetings"));
}

#[test]
fn a_real_root_is_a_folder_not_a_prefix_of_the_dev_one() {
    let real = Path::new("/Users/someone/Library/Application Support/wispr-free");
    let dev = Path::new("/Users/someone/Library/Application Support/wispr-free-dev");

    let paths = Paths::under(dev, "/app/wf-engine-host".into());

    assert!(paths.check(&[real.to_owned()]).is_ok());
    assert!(
        Paths::under(real, "/app/wf-engine-host".into())
            .check(&[real.to_owned()])
            .is_err()
    );
}
