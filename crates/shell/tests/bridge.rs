//! `Shell` as Swift uses it, on the real flow with no key listener and no microphone. Nothing here
//! starts a dictation.
#![cfg(target_os = "macos")]

use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use wf_core::pill::PillState;
use wf_core::settings::{HistoryRetention, Settings, SettingsStore};
use wf_dictation::{Dictation, Event};
use wf_shell::bridge::{EventSink, SaveOutcome, Shell, ShellError, ShellEvent, Versioned};
use wf_shell::flow::{OnEvent, parts_at};
use wf_shell::paths::Paths;

const WAIT: Duration = Duration::from_secs(10);

struct Recorder(Mutex<Sender<ShellEvent>>);

impl EventSink for Recorder {
    fn on_event(&self, event: ShellEvent) {
        let _ = self.0.lock().unwrap().send(event);
    }
}

struct Rig {
    folder: TempDir,
    shell: Arc<Shell>,
    events: Receiver<ShellEvent>,
    /// Sends the shell what the flow would.
    flow: OnEvent,
}

impl Rig {
    fn new() -> Self {
        Self::on(TempDir::new().unwrap(), "settings.toml")
    }

    fn on(folder: TempDir, settings: &str) -> Self {
        let paths = Paths::under(folder.path(), "/nowhere/wf-engine-host".into());
        let (sender, events) = mpsc::channel();
        let sink = Arc::new(Recorder(Mutex::new(sender)));
        let store = SettingsStore::open(folder.path().join(settings));
        let mut flow = None;
        let shell = Shell::wired(store, sink, |settings, on_event| {
            flow = Some(Arc::clone(&on_event));
            Dictation::start(parts_at(&paths, settings, on_event))
        });
        Self {
            folder,
            shell,
            events,
            flow: flow.unwrap(),
        }
    }

    fn current(&self) -> Versioned {
        self.shell.settings().unwrap()
    }

    fn save(&self, settings: Settings, revision: u64) -> Result<SaveOutcome, ShellError> {
        now(Arc::clone(&self.shell).save(settings, revision))
    }

    /// Applies `change` to the latest settings until a save goes through, as Swift's `edit` does.
    fn edit(&self, change: impl Fn(&mut Settings)) {
        edit(&self.shell, change);
    }

    fn next_event(&self) -> ShellEvent {
        self.events.recv_timeout(WAIT).expect("an event")
    }

    /// What arrived up to now.
    fn drain(&self) -> Vec<ShellEvent> {
        self.events.try_iter().collect()
    }
}

fn edit(shell: &Arc<Shell>, change: impl Fn(&mut Settings)) {
    let mut latest = shell.settings().unwrap();
    loop {
        change(&mut latest.settings);
        match now(Arc::clone(shell).save(latest.settings, latest.revision)).unwrap() {
            SaveOutcome::Saved { .. } => return,
            SaveOutcome::Stale { current } => latest = current,
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

fn add_word(word: &str) -> impl Fn(&mut Settings) {
    move |settings| {
        settings.style.add_word(word);
    }
}

fn wait_until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        thread::sleep(Duration::from_millis(10));
    }
}

fn file_settings(path: &Path) -> Settings {
    SettingsStore::open(path).get()
}

#[test]
fn a_save_reaches_the_disk_and_moves_the_revision_on() {
    let rig = Rig::new();
    let start = rig.current();
    let mut changed = start.settings.clone();
    changed.general.show_pill = false;

    let saved = rig.save(changed.clone(), start.revision).unwrap();

    let current = Versioned {
        settings: changed.clone(),
        revision: start.revision + 1,
    };
    assert_eq!(saved, SaveOutcome::Saved { current });
    assert_eq!(
        file_settings(&rig.folder.path().join("settings.toml")),
        changed
    );
}

#[test]
fn a_save_from_an_old_revision_changes_nothing_and_hands_back_the_latest() {
    let rig = Rig::new();
    let old = rig.current();
    rig.edit(|settings| settings.general.show_pill = false);
    let latest = rig.current();
    rig.drain();

    let mut late = old.settings.clone();
    late.media.sounds = false;
    let outcome = rig.save(late, old.revision).unwrap();

    assert_eq!(
        outcome,
        SaveOutcome::Stale {
            current: latest.clone()
        }
    );
    assert_eq!(rig.current(), latest);
    assert_eq!(
        file_settings(&rig.folder.path().join("settings.toml")),
        latest.settings
    );
    assert_eq!(rig.drain(), []);
}

#[test]
fn every_save_tells_the_app_what_is_current() {
    let rig = Rig::new();

    rig.edit(|settings| settings.general.show_pill = false);

    assert_eq!(
        rig.next_event(),
        ShellEvent::SettingsChanged {
            current: rig.current()
        }
    );
}

#[test]
fn a_learned_word_makes_an_earlier_read_stale_and_nothing_is_lost() {
    let rig = Rig::new();
    let read_before = rig.current();

    (rig.flow)(Event::WordLearned("Wispr".to_owned()));

    let mut edited = read_before.settings.clone();
    edited.general.show_pill = false;
    let SaveOutcome::Stale { current } = rig.save(edited, read_before.revision).unwrap() else {
        panic!("a save from before the word went through");
    };
    let mut retried = current.settings;
    retried.general.show_pill = false;
    rig.save(retried, current.revision).unwrap();
    let settings = rig.current().settings;
    assert_eq!(settings.style.dictionary, ["Wispr"]);
    assert!(!settings.general.show_pill);
}

#[test]
fn a_learned_word_is_announced_once() {
    let rig = Rig::new();

    (rig.flow)(Event::WordLearned("Wispr".to_owned()));
    (rig.flow)(Event::WordLearned("Wispr".to_owned()));

    let toasts: Vec<_> = rig
        .drain()
        .into_iter()
        .filter_map(|event| match event {
            ShellEvent::Toast { text } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(toasts, ["Added “Wispr” to the dictionary"]);
}

#[test]
fn saves_racing_each_other_and_the_flow_all_land() {
    let rig = Rig::new();

    thread::scope(|scope| {
        for writer in 0..4 {
            let shell = Arc::clone(&rig.shell);
            scope.spawn(move || {
                for word in 0..10 {
                    edit(&shell, add_word(&format!("saved{writer}x{word}")));
                }
            });
        }
        let flow = Arc::clone(&rig.flow);
        scope.spawn(move || {
            for word in 0..10 {
                flow(Event::WordLearned(format!("learned{word}")));
            }
        });
    });

    assert_eq!(rig.current().settings.style.dictionary.len(), 50);
    let on_disk = file_settings(&rig.folder.path().join("settings.toml"));
    assert_eq!(on_disk, rig.current().settings);
}

#[test]
fn announced_revisions_only_go_up() {
    let rig = Rig::new();

    thread::scope(|scope| {
        for writer in 0..4 {
            let shell = Arc::clone(&rig.shell);
            scope.spawn(move || {
                for word in 0..10 {
                    edit(&shell, add_word(&format!("saved{writer}x{word}")));
                }
            });
        }
    });

    let revisions: Vec<_> = rig
        .drain()
        .into_iter()
        .filter_map(|event| match event {
            ShellEvent::SettingsChanged { current } => Some(current.revision),
            _ => None,
        })
        .collect();
    assert_eq!(revisions, (1..=40).collect::<Vec<_>>());
}

#[test]
fn a_save_the_disk_refuses_is_an_error_and_holds_for_this_run() {
    let folder = TempDir::new().unwrap();
    std::fs::write(folder.path().join("not-a-folder"), "").unwrap();
    let rig = Rig::on(folder, "not-a-folder/settings.toml");
    let start = rig.current();
    let mut changed = start.settings.clone();
    changed.general.show_pill = false;

    let saved = rig.save(changed.clone(), start.revision);

    assert!(matches!(saved, Err(ShellError::Disk { .. })));
    assert_eq!(rig.current().settings, changed);
}

#[test]
fn a_learned_word_the_disk_refuses_says_so() {
    let folder = TempDir::new().unwrap();
    std::fs::write(folder.path().join("not-a-folder"), "").unwrap();
    let rig = Rig::on(folder, "not-a-folder/settings.toml");

    (rig.flow)(Event::WordLearned("Wispr".to_owned()));

    assert!(rig.drain().iter().any(
        |event| matches!(event, ShellEvent::Toast { text } if text.starts_with("Couldn't save settings:"))
    ));
}

#[test]
fn a_save_reaches_the_running_flow() {
    let rig = Rig::new();
    let history = rig.folder.path().join("history.sqlite3");
    wait_until("the history opens", || history.exists());

    rig.edit(|settings| settings.privacy.history_retention = HistoryRetention::Never);

    wait_until("the flow erases the history", || !history.exists());
}

#[test]
fn what_the_flow_reports_crosses_as_shell_events() {
    let rig = Rig::new();
    rig.drain();
    let cases = [
        (
            Event::Pill(PillState::Error {
                reason: "no mic".to_owned(),
            }),
            ShellEvent::Pill {
                state: PillState::Error {
                    reason: "no mic".to_owned(),
                },
            },
        ),
        (Event::Level(0.5), ShellEvent::Level { value: 0.5 }),
        (
            Event::Toast("hello".to_owned()),
            ShellEvent::Toast {
                text: "hello".to_owned(),
            },
        ),
        (
            Event::Recent(vec!["one".to_owned()]),
            ShellEvent::Recent {
                transcripts: vec!["one".to_owned()],
            },
        ),
        (Event::Meeting(true), ShellEvent::Meeting { active: true }),
        (
            Event::SuggestMeeting("Zoom".to_owned()),
            ShellEvent::SuggestMeeting {
                app: "Zoom".to_owned(),
            },
        ),
        (
            Event::NoTextField("text".to_owned()),
            ShellEvent::NoTextField {
                text: "text".to_owned(),
            },
        ),
    ];

    for (event, expected) in cases {
        (rig.flow)(event);

        assert_eq!(rig.next_event(), expected);
    }
}

#[test]
fn a_flow_event_after_the_shell_went_away_goes_nowhere() {
    let Rig { flow, shell, .. } = Rig::new();
    drop(shell);

    flow(Event::Toast("late".to_owned()));
}

#[test]
fn quitting_with_nothing_running_finishes_at_once() {
    let rig = Rig::new();

    let started = Instant::now();
    now(rig.shell.quit()).unwrap();

    assert!(started.elapsed() < Duration::from_secs(1));
}
