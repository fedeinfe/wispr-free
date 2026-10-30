//! Context (see GLOSSARY): the focused app, its window title and the text around the cursor,
//! read through the accessibility APIs (spec §3, §7: AX / UI Automation / AT-SPI).

use std::ops::Range;

#[cfg(any(target_os = "macos", test))]
mod ax_role;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "windows", test))]
mod uia_role;
#[cfg(target_os = "macos")]
pub use macos::{SystemContext, accessibility_trusted};
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use self::windows::{SystemContext, accessibility_trusted};
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{SystemContext, accessibility_trusted};

/// Who has keyboard focus. `id` is the bundle id (macOS), executable name (Windows) or
/// WM_CLASS / app name (Linux).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppIdentity {
    pub id: String,
    pub name: Option<String>,
    pub pid: Option<u32>,
}

/// What kind of element has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// A text field, text area, or web/editor content that accepts typing.
    Editable,
    /// A password field: paste works, but its text is never read.
    Secure,
    /// Something focused that clearly isn't text (a button, a list, a page with no field).
    NotEditable,
    /// No focused element at all.
    Nothing,
    /// The accessibility API couldn't tell (no permission, or the app exposes nothing).
    Unknown,
}

/// The text around the cursor, each side capped at [`CONTEXT_CHARS`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextAround {
    pub before: String,
    pub selection: String,
    pub after: String,
}

/// Characters kept on each side of the cursor (spec §3).
pub const CONTEXT_CHARS: usize = 500;

/// Cap on the selected text, so a select-all in a long document stays cheap.
pub const MAX_SELECTION_CHARS: usize = 2_000;

impl TextAround {
    /// Keeps the [`CONTEXT_CHARS`] characters nearest the cursor on each side.
    pub fn new(before: &str, selection: &str, after: &str) -> Self {
        let skip = before.chars().count().saturating_sub(CONTEXT_CHARS);
        Self {
            before: before.chars().skip(skip).collect(),
            selection: selection.chars().take(MAX_SELECTION_CHARS).collect(),
            after: after.chars().take(CONTEXT_CHARS).collect(),
        }
    }

    /// Like [`TextAround::new`] for UTF-16 slices read with a [`Utf16Window`], whose edges
    /// may split a surrogate pair; the broken halves are dropped.
    pub fn from_utf16(before: &[u16], selection: &[u16], after: &[u16]) -> Self {
        Self::new(&decode(before), &decode(selection), &decode(after))
    }
}

/// The UTF-16 ranges to read around a selection (AX indexes text in UTF-16 units).
/// Each side is twice the character cap, which covers it even for all-surrogate text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Utf16Window {
    pub before: Range<usize>,
    pub selection: Range<usize>,
    pub after: Range<usize>,
}

impl Utf16Window {
    pub fn around(selection: Range<usize>, text_len: usize) -> Self {
        let end = selection.end.min(text_len);
        let start = selection.start.min(end);
        Self {
            before: start.saturating_sub(2 * CONTEXT_CHARS)..start,
            selection: start..end.min(start + 2 * MAX_SELECTION_CHARS),
            after: end..(end + 2 * CONTEXT_CHARS).min(text_len),
        }
    }
}

fn decode(units: &[u16]) -> String {
    char::decode_utf16(units.iter().copied())
        .filter_map(Result::ok)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    pub app: Option<AppIdentity>,
    pub window_title: Option<String>,
    pub focus: Focus,
    /// `None` when not asked for, unreadable, or the field is secure.
    pub text: Option<TextAround>,
}

/// How much to read: insertion only needs the app and focus kind, cheaply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextDepth {
    FocusOnly,
    WithText,
}

/// Reads the current context. Never fails: what can't be read comes back as `None` /
/// [`Focus::Unknown`].
pub trait ContextSource {
    fn read(&self, depth: ContextDepth) -> Context;
}
