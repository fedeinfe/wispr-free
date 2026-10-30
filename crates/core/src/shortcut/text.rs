//! Keys as text: stable names for the settings file and labels for the Settings window.

use std::fmt;
use std::str::FromStr;

use super::{Action, Combo, Key, MouseButton, Platform, ShortcutError, Side};

/// Named keys without a payload, as they appear in the settings file.
const NAMED_KEYS: [(Key, &str); 17] = [
    (Key::Fn, "Fn"),
    (Key::Esc, "Esc"),
    (Key::Space, "Space"),
    (Key::Enter, "Enter"),
    (Key::Tab, "Tab"),
    (Key::Backspace, "Backspace"),
    (Key::Delete, "Delete"),
    (Key::CapsLock, "CapsLock"),
    (Key::ArrowUp, "ArrowUp"),
    (Key::ArrowDown, "ArrowDown"),
    (Key::ArrowLeft, "ArrowLeft"),
    (Key::ArrowRight, "ArrowRight"),
    (Key::Home, "Home"),
    (Key::End, "End"),
    (Key::PageUp, "PageUp"),
    (Key::PageDown, "PageDown"),
    (Key::Insert, "Insert"),
];

/// The settings-file name of a key.
impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some((_, name)) = NAMED_KEYS.iter().find(|(key, _)| key == self) {
            return f.write_str(name);
        }
        let side = |side| match side {
            Side::Left => "Left",
            Side::Right => "Right",
            Side::Either => "",
        };
        match *self {
            Key::Ctrl(s) => write!(f, "{}Ctrl", side(s)),
            Key::Shift(s) => write!(f, "{}Shift", side(s)),
            Key::Alt(s) => write!(f, "{}Alt", side(s)),
            Key::Meta(s) => write!(f, "{}Meta", side(s)),
            Key::PrintScreen => f.write_str("PrintScreen"),
            Key::Char(c) => write!(f, "{c}"),
            Key::F(n) => write!(f, "F{n}"),
            Key::Mouse(MouseButton::Middle) => f.write_str("MouseMiddle"),
            Key::Mouse(MouseButton::Back) => f.write_str("MouseBack"),
            Key::Mouse(MouseButton::Forward) => f.write_str("MouseForward"),
            Key::Mouse(MouseButton::Other(n)) => write!(f, "Mouse{n}"),
            Key::Other(code) => write!(f, "Code{code}"),
            _ => unreachable!("named keys are handled above"),
        }
    }
}

/// A string that names no key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown key name {0:?}")]
pub struct UnknownKey(String);

impl FromStr for Key {
    type Err = UnknownKey;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        parse(name).ok_or_else(|| UnknownKey(name.to_owned()))
    }
}

fn parse(name: &str) -> Option<Key> {
    if let Some(&(key, _)) = NAMED_KEYS.iter().find(|(_, n)| *n == name) {
        return Some(key);
    }
    if name == "PrintScreen" {
        return Some(Key::PrintScreen);
    }
    let (side, modifier) = match (name.strip_prefix("Left"), name.strip_prefix("Right")) {
        (Some(rest), _) => (Side::Left, rest),
        (_, Some(rest)) => (Side::Right, rest),
        _ => (Side::Either, name),
    };
    match modifier {
        "Ctrl" => return Some(Key::Ctrl(side)),
        "Shift" => return Some(Key::Shift(side)),
        "Alt" => return Some(Key::Alt(side)),
        "Meta" => return Some(Key::Meta(side)),
        _ => {}
    }
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return is_char_key(c).then_some(Key::Char(c));
    }
    if let Some(n) = name.strip_prefix('F').and_then(|n| n.parse().ok()) {
        return (1..=24).contains(&n).then_some(Key::F(n));
    }
    if let Some(button) = name.strip_prefix("Mouse") {
        return match button {
            "Middle" => Some(MouseButton::Middle),
            "Back" => Some(MouseButton::Back),
            "Forward" => Some(MouseButton::Forward),
            n => n.parse().ok().map(MouseButton::Other),
        }
        .map(Key::Mouse);
    }
    name.strip_prefix("Code")?.parse().ok().map(Key::Other)
}

fn is_char_key(c: char) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit() || r"-=[]\;',./`".contains(c)
}

impl TryFrom<String> for Key {
    type Error = UnknownKey;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        name.parse()
    }
}

impl From<Key> for String {
    fn from(key: Key) -> Self {
        key.to_string()
    }
}

impl Key {
    /// What the Settings window calls this key on `platform`.
    pub fn label(self, platform: Platform) -> String {
        let sided = |name: &str, side| match side {
            Side::Left => format!("Left {name}"),
            Side::Right => format!("Right {name}"),
            Side::Either => name.to_owned(),
        };
        match self {
            Key::Ctrl(side) => sided("Ctrl", side),
            Key::Shift(side) => sided("Shift", side),
            Key::Alt(side) if platform == Platform::MacOs => sided("Opt", side),
            Key::Alt(side) => sided("Alt", side),
            Key::Meta(side) => sided(
                match platform {
                    Platform::MacOs => "Cmd",
                    Platform::Windows => "Win",
                    Platform::Linux => "Super",
                },
                side,
            ),
            Key::ArrowUp => "Up".to_owned(),
            Key::ArrowDown => "Down".to_owned(),
            Key::ArrowLeft => "Left".to_owned(),
            Key::ArrowRight => "Right".to_owned(),
            Key::CapsLock => "Caps Lock".to_owned(),
            Key::PageUp => "Page Up".to_owned(),
            Key::PageDown => "Page Down".to_owned(),
            Key::PrintScreen => "Print Screen".to_owned(),
            Key::Mouse(MouseButton::Middle) => "Mouse Middle".to_owned(),
            Key::Mouse(MouseButton::Back) => "Mouse Back".to_owned(),
            Key::Mouse(MouseButton::Forward) => "Mouse Forward".to_owned(),
            Key::Mouse(MouseButton::Other(n)) => format!("Mouse {n}"),
            Key::Other(code) => format!("Key {code}"),
            key => key.to_string(),
        }
    }
}

impl Combo {
    /// What the Settings window shows for this combo on `platform`, e.g. `Ctrl+Cmd+V`.
    pub fn label(&self, platform: Platform) -> String {
        let labels: Vec<_> = self.keys().iter().map(|key| key.label(platform)).collect();
        labels.join("+")
    }
}

impl Action {
    /// What the Settings window calls this action.
    pub fn label(self) -> &'static str {
        match self {
            Action::PushToTalk => "Push-to-talk",
            Action::HandsFree => "Hands-free",
            Action::Cancel => "Cancel",
            Action::PasteLast => "Paste last transcript",
            Action::CopyLast => "Copy last transcript",
            Action::MeetingNotes => "Start/stop meeting notes",
        }
    }
}

impl ShortcutError {
    /// Why the recorder refused a combo, in words for the user.
    pub fn message(self) -> String {
        match self {
            ShortcutError::Empty => "Press a shortcut.".to_owned(),
            ShortcutError::NeedsModifier => {
                "Include a modifier key or a non-primary mouse button.".to_owned()
            }
            ShortcutError::TooManyKeys => "Use at most 3 keys.".to_owned(),
            ShortcutError::BothSidesOfModifier => {
                "Don't use the left and right side of the same modifier.".to_owned()
            }
            ShortcutError::Conflict(action) => {
                format!("Already used by {}.", action.label())
            }
            ShortcutError::Reserved => "Reserved by the system.".to_owned(),
        }
    }
}
