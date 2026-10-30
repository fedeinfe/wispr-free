//! Insertion (see GLOSSARY): clipboard save → transient write → paste keystroke → restore
//! after ~500 ms (spec §3). The orchestration is OS-free; backends implement the traits.

use std::time::Duration;

use crate::context::{AppIdentity, ContextDepth, ContextSource, Focus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    MacOs,
    Windows,
    Linux,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteChord {
    CmdV,
    CtrlV,
    ShiftInsert,
}

/// The paste keystroke: Cmd+V on macOS (terminals included), Ctrl+V elsewhere, and
/// Shift+Insert in Windows/Linux terminals, where Ctrl+V is a literal control character.
pub fn paste_chord(os: Os, app: Option<&AppIdentity>) -> PasteChord {
    match os {
        Os::MacOs => PasteChord::CmdV,
        Os::Windows | Os::Linux if app.is_some_and(is_terminal) => PasteChord::ShiftInsert,
        Os::Windows | Os::Linux => PasteChord::CtrlV,
    }
}

/// Terminal emulators, matched on the last dotted segment of the app id, without `.exe`.
const TERMINALS: &[&str] = &[
    "terminal",
    "iterm2",
    "warp-stable",
    "ghostty",
    "kitty",
    "alacritty",
    "wezterm",
    "wezterm-gui",
    "hyper",
    "tabby",
    "windowsterminal",
    "openconsole",
    "conhost",
    "cmd",
    "powershell",
    "pwsh",
    "mintty",
    "gnome-terminal",
    "gnome-terminal-server",
    "kgx",
    "ptyxis",
    "konsole",
    "xterm",
    "uxterm",
    "urxvt",
    "rxvt",
    "st",
    "tilix",
    "xfce4-terminal",
    "terminator",
    "foot",
    "footclient",
    "sakura",
    "lxterminal",
    "mate-terminal",
    "qterminal",
    "terminology",
    "guake",
    "tilda",
    "yakuake",
];

pub fn is_terminal(app: &AppIdentity) -> bool {
    let id = app.id.to_lowercase();
    let id = id.strip_suffix(".exe").unwrap_or(&id);
    let name = id.rsplit('.').next().unwrap_or(id);
    TERMINALS.contains(&name)
}

#[derive(Debug, thiserror::Error)]
pub enum InsertError {
    #[error("accessibility permission not granted")]
    PermissionDenied,
    #[error("clipboard: {0}")]
    Clipboard(String),
    #[error("keystroke: {0}")]
    Keystroke(String),
}

/// One pasteboard item as (format, bytes) pairs. Format names are backend-specific
/// (UTIs on macOS, MIME-like names elsewhere).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClipboardItem {
    pub formats: Vec<(String, Vec<u8>)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClipboardSnapshot {
    pub items: Vec<ClipboardItem>,
}

/// Identifies the clipboard state right after a write, to detect a later user copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangeMark(pub u64);

pub trait Clipboard {
    fn snapshot(&mut self) -> Result<ClipboardSnapshot, InsertError>;
    /// Writes plain text; `transient` flags it so clipboard managers skip it.
    fn write_text(&mut self, text: &str, transient: bool) -> Result<ChangeMark, InsertError>;
    /// True when something else was copied after the write that returned `mark`.
    fn changed_since(&self, mark: ChangeMark) -> bool;
    fn restore(&mut self, snapshot: ClipboardSnapshot) -> Result<(), InsertError>;
}

pub trait PasteKeys {
    fn send(&mut self, chord: PasteChord) -> Result<(), InsertError>;
}

pub trait Sleep {
    fn sleep(&mut self, duration: Duration);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timings {
    /// Pause between the clipboard write and the keystroke.
    pub settle: Duration,
    /// Pause between the keystroke and the restore, so the app has read the clipboard.
    pub restore_after: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertOutcome {
    Pasted {
        chord: PasteChord,
        clipboard_restored: bool,
    },
    /// The text was left on the clipboard instead; the UI shows the "Copied — paste with …"
    /// toast and keeps it as the last transcript.
    Copied { reason: CopyReason },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyReason {
    NoFocusedField,
    /// The keystroke couldn't be sent (usually missing Accessibility permission).
    PasteFailed(String),
}

/// Paste unless there's clearly no text field. Terminals draw their own text view, which
/// often reports as a plain group/window, so they always get the paste.
fn accepts_paste(focus: Focus, app: Option<&AppIdentity>) -> bool {
    match focus {
        Focus::Editable | Focus::Secure | Focus::Unknown => true,
        Focus::NotEditable => app.is_some_and(is_terminal),
        Focus::Nothing => false,
    }
}

pub struct Inserter<C, K, X, S> {
    clipboard: C,
    keys: K,
    context: X,
    sleep: S,
    os: Os,
    timings: Timings,
}

impl<C: Clipboard, K: PasteKeys, X: ContextSource, S: Sleep> Inserter<C, K, X, S> {
    pub fn new(clipboard: C, keys: K, context: X, sleep: S, os: Os, timings: Timings) -> Self {
        Self {
            clipboard,
            keys,
            context,
            sleep,
            os,
            timings,
        }
    }

    pub fn insert(&mut self, text: &str) -> Result<InsertOutcome, InsertError> {
        let context = self.context.read(ContextDepth::FocusOnly);
        if !accepts_paste(context.focus, context.app.as_ref()) {
            self.clipboard.write_text(text, false)?;
            return Ok(InsertOutcome::Copied {
                reason: CopyReason::NoFocusedField,
            });
        }
        // An unreadable clipboard (e.g. lazily promised data) must not block dictation.
        let saved = self.clipboard.snapshot().ok();
        let mark = self.clipboard.write_text(text, true)?;
        self.sleep.sleep(self.timings.settle);
        let chord = paste_chord(self.os, context.app.as_ref());
        if let Err(err) = self.keys.send(chord) {
            // Keep the dictated text rather than the old clipboard: the user can still paste it.
            self.clipboard.write_text(text, false)?;
            return Ok(InsertOutcome::Copied {
                reason: CopyReason::PasteFailed(err.to_string()),
            });
        }
        self.sleep.sleep(self.timings.restore_after);
        let clipboard_restored = match saved {
            Some(saved) if !self.clipboard.changed_since(mark) => {
                self.clipboard.restore(saved).is_ok()
            }
            _ => false,
        };
        Ok(InsertOutcome::Pasted {
            chord,
            clipboard_restored,
        })
    }
}
