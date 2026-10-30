//! Insertion orchestration (spec §3 "Insertion") driven through fakes: the clipboard swap,
//! the paste keystroke, the delayed restore and the fallbacks, without touching the OS.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use wf_platform::context::{AppIdentity, Context, ContextDepth, ContextSource, Focus};
use wf_platform::insert::{
    ChangeMark, Clipboard, ClipboardItem, ClipboardSnapshot, CopyReason, InsertError,
    InsertOutcome, Inserter, Os, PasteChord, PasteKeys, Sleep, Timings,
};

#[derive(Debug, Clone, PartialEq)]
enum Event {
    Snapshot,
    Write { text: String, transient: bool },
    Paste(PasteChord),
    Sleep(Duration),
    Restore(ClipboardSnapshot),
}

type Log = Rc<RefCell<Vec<Event>>>;

struct FakeClipboard {
    log: Log,
    contents: ClipboardSnapshot,
    changes: u64,
    user_copied: Rc<Cell<bool>>,
    unreadable: bool,
    unwritable_restore: bool,
}

impl Clipboard for FakeClipboard {
    fn snapshot(&mut self) -> Result<ClipboardSnapshot, InsertError> {
        self.log.borrow_mut().push(Event::Snapshot);
        if self.unreadable {
            return Err(InsertError::Clipboard("promised data".into()));
        }
        Ok(self.contents.clone())
    }
    fn write_text(&mut self, text: &str, transient: bool) -> Result<ChangeMark, InsertError> {
        self.log.borrow_mut().push(Event::Write {
            text: text.into(),
            transient,
        });
        self.contents = text_snapshot(text);
        self.changes += 1;
        Ok(ChangeMark(self.changes))
    }
    fn changed_since(&self, mark: ChangeMark) -> bool {
        self.user_copied.get() || self.changes != mark.0
    }
    fn restore(&mut self, snapshot: ClipboardSnapshot) -> Result<(), InsertError> {
        if self.unwritable_restore {
            return Err(InsertError::Clipboard("restore failed".into()));
        }
        self.log.borrow_mut().push(Event::Restore(snapshot.clone()));
        self.contents = snapshot;
        self.changes += 1;
        Ok(())
    }
}

struct FakeKeys {
    log: Log,
    denied: bool,
}

impl PasteKeys for FakeKeys {
    fn send(&mut self, chord: PasteChord) -> Result<(), InsertError> {
        if self.denied {
            return Err(InsertError::PermissionDenied);
        }
        self.log.borrow_mut().push(Event::Paste(chord));
        Ok(())
    }
}

struct FakeSleep {
    log: Log,
}

impl Sleep for FakeSleep {
    fn sleep(&mut self, duration: Duration) {
        self.log.borrow_mut().push(Event::Sleep(duration));
    }
}

struct FakeContext {
    app: &'static str,
    focus: Focus,
}

impl ContextSource for FakeContext {
    fn read(&self, _depth: ContextDepth) -> Context {
        Context {
            app: Some(AppIdentity {
                id: self.app.into(),
                name: None,
                pid: None,
            }),
            window_title: None,
            focus: self.focus,
            text: None,
        }
    }
}

fn text_snapshot(text: &str) -> ClipboardSnapshot {
    ClipboardSnapshot {
        items: vec![ClipboardItem {
            formats: vec![("public.utf8-plain-text".into(), text.as_bytes().to_vec())],
        }],
    }
}

const TIMINGS: Timings = Timings {
    settle: Duration::from_millis(20),
    restore_after: Duration::from_millis(500),
};

type FakeInserter = Inserter<FakeClipboard, FakeKeys, FakeContext, FakeSleep>;

fn inserter(os: Os, app: &'static str, focus: Focus) -> (FakeInserter, Log) {
    let (inserter, log, _) = build(os, app, focus, Faults::default());
    (inserter, log)
}

#[derive(Default)]
struct Faults {
    keys_denied: bool,
    clipboard_unreadable: bool,
    restore_fails: bool,
}

/// Returns a flag that simulates the user copying something while the paste is in flight.
fn build(
    os: Os,
    app: &'static str,
    focus: Focus,
    faults: Faults,
) -> (FakeInserter, Log, Rc<Cell<bool>>) {
    let log: Log = Rc::default();
    let user_copied = Rc::<Cell<bool>>::default();
    let inserter = Inserter::new(
        FakeClipboard {
            log: log.clone(),
            contents: text_snapshot("previous"),
            changes: 0,
            user_copied: user_copied.clone(),
            unreadable: faults.clipboard_unreadable,
            unwritable_restore: faults.restore_fails,
        },
        FakeKeys {
            log: log.clone(),
            denied: faults.keys_denied,
        },
        FakeContext { app, focus },
        FakeSleep { log: log.clone() },
        os,
        TIMINGS,
    );
    (inserter, log, user_copied)
}

#[test]
fn pastes_into_the_focused_field_and_restores_the_previous_clipboard() {
    let (mut inserter, log) = inserter(Os::MacOs, "com.apple.TextEdit", Focus::Editable);

    let outcome = inserter.insert("hello world").unwrap();

    assert_eq!(
        outcome,
        InsertOutcome::Pasted {
            chord: PasteChord::CmdV,
            clipboard_restored: true
        }
    );
    assert_eq!(
        *log.borrow(),
        vec![
            Event::Snapshot,
            Event::Write {
                text: "hello world".into(),
                transient: true
            },
            Event::Sleep(Duration::from_millis(20)),
            Event::Paste(PasteChord::CmdV),
            Event::Sleep(Duration::from_millis(500)),
            Event::Restore(text_snapshot("previous")),
        ]
    );
}

#[test]
fn paste_chord_follows_the_os_with_shift_insert_in_terminals() {
    let cases = [
        (Os::MacOs, "com.apple.TextEdit", PasteChord::CmdV),
        (Os::MacOs, "com.apple.Terminal", PasteChord::CmdV),
        (Os::MacOs, "com.googlecode.iterm2", PasteChord::CmdV),
        (Os::Windows, "chrome.exe", PasteChord::CtrlV),
        (Os::Windows, "WindowsTerminal.exe", PasteChord::ShiftInsert),
        (Os::Windows, "conhost.exe", PasteChord::ShiftInsert),
        (Os::Windows, "Code.exe", PasteChord::CtrlV),
        (Os::Linux, "firefox", PasteChord::CtrlV),
        (Os::Linux, "gnome-terminal-server", PasteChord::ShiftInsert),
        (Os::Linux, "org.kde.konsole", PasteChord::ShiftInsert),
        (Os::Linux, "kitty", PasteChord::ShiftInsert),
    ];

    for (os, app, expected) in cases {
        let (mut inserter, _log) = inserter(os, app, Focus::Editable);

        let outcome = inserter.insert("x").unwrap();

        assert_eq!(
            outcome,
            InsertOutcome::Pasted {
                chord: expected,
                clipboard_restored: true
            },
            "{os:?} {app}"
        );
    }
}

#[test]
fn leaves_the_clipboard_alone_when_the_user_copied_something_meanwhile() {
    let (mut inserter, log, user_copied) = build(
        Os::MacOs,
        "com.apple.TextEdit",
        Focus::Editable,
        Faults::default(),
    );
    user_copied.set(true);

    let outcome = inserter.insert("hello").unwrap();

    assert_eq!(
        outcome,
        InsertOutcome::Pasted {
            chord: PasteChord::CmdV,
            clipboard_restored: false
        }
    );
    assert!(!log.borrow().iter().any(|e| matches!(e, Event::Restore(_))));
}

#[test]
fn without_a_focused_text_field_the_text_is_left_on_the_clipboard() {
    for focus in [Focus::Nothing, Focus::NotEditable] {
        let (mut inserter, log) = inserter(Os::MacOs, "com.apple.finder", focus);

        let outcome = inserter.insert("hello").unwrap();

        assert_eq!(
            outcome,
            InsertOutcome::Copied {
                reason: CopyReason::NoFocusedField
            },
            "{focus:?}"
        );
        assert_eq!(
            *log.borrow(),
            vec![Event::Write {
                text: "hello".into(),
                transient: false
            }],
            "{focus:?}"
        );
    }
}

#[test]
fn pastes_when_focus_is_unknown_secure_or_a_terminal_view() {
    let cases = [
        ("com.apple.TextEdit", Focus::Unknown),
        ("com.apple.Safari", Focus::Secure),
        ("io.alacritty", Focus::NotEditable),
    ];

    for (app, focus) in cases {
        let (mut inserter, _log) = inserter(Os::MacOs, app, focus);

        let outcome = inserter.insert("hello").unwrap();

        assert!(
            matches!(outcome, InsertOutcome::Pasted { .. }),
            "{app} {focus:?}: {outcome:?}"
        );
    }
}

#[test]
fn when_the_keystroke_cannot_be_sent_the_text_stays_on_the_clipboard() {
    let (mut inserter, log, _) = build(
        Os::MacOs,
        "com.apple.TextEdit",
        Focus::Editable,
        Faults {
            keys_denied: true,
            ..Faults::default()
        },
    );

    let outcome = inserter.insert("hello").unwrap();

    assert_eq!(
        outcome,
        InsertOutcome::Copied {
            reason: CopyReason::PasteFailed("accessibility permission not granted".into())
        }
    );
    let log = log.borrow();
    assert!(!log.iter().any(|e| matches!(e, Event::Restore(_))));
    assert_eq!(
        log.last(),
        Some(&Event::Write {
            text: "hello".into(),
            transient: false
        })
    );
}

#[test]
fn an_unreadable_clipboard_does_not_block_the_paste() {
    let faults = Faults {
        clipboard_unreadable: true,
        ..Faults::default()
    };
    let (mut inserter, log, _) = build(Os::MacOs, "com.apple.TextEdit", Focus::Editable, faults);

    let outcome = inserter.insert("hello").unwrap();

    assert_eq!(
        outcome,
        InsertOutcome::Pasted {
            chord: PasteChord::CmdV,
            clipboard_restored: false
        }
    );
    assert!(log.borrow().contains(&Event::Paste(PasteChord::CmdV)));
    assert!(!log.borrow().iter().any(|e| matches!(e, Event::Restore(_))));
}

#[test]
fn a_failed_restore_still_reports_the_paste() {
    let faults = Faults {
        restore_fails: true,
        ..Faults::default()
    };
    let (mut inserter, _log, _) = build(Os::MacOs, "com.apple.TextEdit", Focus::Editable, faults);

    let outcome = inserter.insert("hello").unwrap();

    assert_eq!(
        outcome,
        InsertOutcome::Pasted {
            chord: PasteChord::CmdV,
            clipboard_restored: false
        }
    );
}
