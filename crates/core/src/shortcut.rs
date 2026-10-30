//! The shortcut model (spec §2): keys, combos, the per-action bindings with their per-OS defaults,
//! and the recorder's validation rules (reused by Settings in ticket 12).

/// Which physical side of a modifier. Pressed keys are always `Left` or `Right`; a binding may say
/// `Either` so that both sides trigger it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
    Either,
}

/// Mouse buttons usable in shortcuts. The primary (left) and secondary (right) buttons never are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Middle,
    Back,
    Forward,
    /// Further buttons, numbered from 0 = primary as the OS reports them.
    Other(u8),
}

/// A physical key, independent of the keyboard layout. `Meta` is Cmd on macOS, Win on Windows and
/// Super on Linux; `Alt` is Option on macOS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Fn,
    Ctrl(Side),
    Shift(Side),
    Alt(Side),
    Meta(Side),
    Esc,
    Space,
    Enter,
    Tab,
    Backspace,
    Delete,
    CapsLock,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    PrintScreen,
    /// A key named by its character on a US layout: `'A'..='Z'`, `'0'..='9'` and ``-=[]\;',./` ``.
    Char(char),
    /// Function keys F1–F24.
    F(u8),
    Mouse(MouseButton),
    /// A key the backend has no name for, by its native code. Still counts as a key press.
    Other(u32),
}

impl Key {
    pub fn is_modifier(self) -> bool {
        matches!(
            self,
            Key::Fn | Key::Ctrl(_) | Key::Shift(_) | Key::Alt(_) | Key::Meta(_)
        )
    }

    /// Whether this binding key is triggered by the `pressed` key.
    pub fn matches(self, pressed: Key) -> bool {
        match (self.modifier_side(), pressed.modifier_side()) {
            (Some((kind, Side::Either)), Some((pressed_kind, _))) => kind == pressed_kind,
            _ => self == pressed,
        }
    }

    /// Whether a single press could trigger both binding keys.
    fn overlaps(self, other: Key) -> bool {
        self.matches(other) || other.matches(self)
    }

    /// For a sided modifier: a side-less discriminant and its side.
    fn modifier_side(self) -> Option<(u8, Side)> {
        match self {
            Key::Ctrl(side) => Some((0, side)),
            Key::Shift(side) => Some((1, side)),
            Key::Alt(side) => Some((2, side)),
            Key::Meta(side) => Some((3, side)),
            _ => None,
        }
    }
}

/// One press or release of a key, as reported by a platform backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: Key,
    pub pressed: bool,
}

impl KeyEvent {
    pub fn press(key: Key) -> Self {
        Self { key, pressed: true }
    }

    pub fn release(key: Key) -> Self {
        Self {
            key,
            pressed: false,
        }
    }
}

/// The keys of one shortcut, held together.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Combo(Vec<Key>);

impl Combo {
    pub fn new(keys: impl Into<Vec<Key>>) -> Self {
        Self(keys.into())
    }

    pub fn keys(&self) -> &[Key] {
        &self.0
    }

    /// Whether `pressed` holds exactly this combo: every key matched, nothing extra.
    pub fn is_held_exactly(&self, held: &[Key]) -> bool {
        held.len() == self.0.len() && self.is_held_within(held)
    }

    /// Whether every key of this combo is among `held`.
    pub fn is_held_within(&self, held: &[Key]) -> bool {
        !self.0.is_empty()
            && self
                .0
                .iter()
                .all(|key| held.iter().any(|&pressed| key.matches(pressed)))
    }

    /// Whether `pressed` is one of this combo's keys.
    pub fn contains(&self, pressed: Key) -> bool {
        self.0.iter().any(|key| key.matches(pressed))
    }

    /// Whether the same keys could trigger both combos.
    fn collides_with(&self, other: &[Key]) -> bool {
        self.0.len() == other.len()
            && self
                .0
                .iter()
                .all(|key| other.iter().any(|&o| key.overlaps(o)))
    }
}

/// Why the recorder refuses a combo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutError {
    Empty,
    NeedsModifier,
    TooManyKeys,
    BothSidesOfModifier,
    /// The combo is already bound to this action.
    Conflict(Action),
    /// The OS or nearly every app uses it (copy, paste, quit, app switch, screenshot…).
    Reserved,
}

/// The most keys a combo may have.
pub const MAX_COMBO_KEYS: usize = 3;

/// The recorder's rules (spec §2): whether `combo` may be bound to `action`, given the other
/// actions' `bindings`.
pub fn validate(
    action: Action,
    combo: &Combo,
    bindings: &Bindings,
    platform: Platform,
) -> Result<(), ShortcutError> {
    let keys = combo.keys();
    if keys.is_empty() {
        return Err(ShortcutError::Empty);
    }
    if keys.len() > MAX_COMBO_KEYS {
        return Err(ShortcutError::TooManyKeys);
    }
    let has_trigger = keys
        .iter()
        .any(|key| key.is_modifier() || matches!(key, Key::Mouse(_)));
    if !has_trigger && !(action == Action::Cancel && keys == [Key::Esc]) {
        return Err(ShortcutError::NeedsModifier);
    }
    let both_sides = keys.iter().enumerate().any(|(i, key)| {
        let kind = key.modifier_side().map(|(kind, _)| kind);
        kind.is_some()
            && keys[i + 1..]
                .iter()
                .any(|other| other.modifier_side().map(|(kind, _)| kind) == kind)
    });
    if both_sides {
        return Err(ShortcutError::BothSidesOfModifier);
    }
    if let Some(other) = Action::ALL
        .into_iter()
        .find(|&other| other != action && combo.collides_with(bindings.get(other).keys()))
    {
        return Err(ShortcutError::Conflict(other));
    }
    if reserved_combos(platform)
        .iter()
        .any(|reserved| combo.collides_with(reserved))
    {
        return Err(ShortcutError::Reserved);
    }
    Ok(())
}

/// Combos the OS or nearly every app already owns. Not exhaustive: the common ones users hit.
fn reserved_combos(platform: Platform) -> &'static [&'static [Key]] {
    use Key::{Alt, Char, Ctrl, Delete, Esc, F, Meta, PrintScreen, Shift, Space, Tab};
    const CTRL: Key = Ctrl(Side::Either);
    const SHIFT: Key = Shift(Side::Either);
    const ALT: Key = Alt(Side::Either);
    const META: Key = Meta(Side::Either);
    match platform {
        Platform::MacOs => &[
            &[META, Char('A')],
            &[META, Char('C')],
            &[META, Char('V')],
            &[META, Char('X')],
            &[META, Char('Z')],
            &[META, Char('S')],
            &[META, Char('W')],
            &[META, Char('Q')],
            &[META, Char('H')],
            &[META, Char('M')],
            &[META, Tab],
            &[META, Char('`')],
            &[META, Space],
            &[CTRL, Space],
            &[META, ALT, Esc],
            &[META, SHIFT, Char('3')],
            &[META, SHIFT, Char('4')],
            &[META, SHIFT, Char('5')],
        ],
        Platform::Windows => &[
            &[CTRL, Char('A')],
            &[CTRL, Char('C')],
            &[CTRL, Char('V')],
            &[CTRL, Char('X')],
            &[CTRL, Char('Z')],
            &[CTRL, Char('S')],
            &[CTRL, Char('W')],
            &[CTRL, Esc],
            &[CTRL, SHIFT, Esc],
            &[CTRL, ALT, Delete],
            &[ALT, Tab],
            &[ALT, Esc],
            &[ALT, F(4)],
            &[ALT, PrintScreen],
            &[META, Tab],
            &[META, Space],
            &[META, Char('D')],
            &[META, Char('E')],
            &[META, Char('L')],
            &[META, Char('R')],
            &[META, SHIFT, Char('S')],
        ],
        Platform::Linux => &[
            &[CTRL, Char('A')],
            &[CTRL, Char('C')],
            &[CTRL, Char('V')],
            &[CTRL, Char('X')],
            &[CTRL, Char('Z')],
            &[CTRL, Char('S')],
            &[CTRL, Char('W')],
            &[CTRL, Char('Q')],
            &[CTRL, ALT, Delete],
            &[CTRL, ALT, Char('T')],
            &[ALT, Tab],
            &[ALT, F(4)],
            &[ALT, PrintScreen],
            &[SHIFT, PrintScreen],
            &[META, Tab],
            &[META, Char('L')],
        ],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    MacOs,
    Windows,
    Linux,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Platform::MacOs
        } else if cfg!(target_os = "windows") {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    PushToTalk,
    HandsFree,
    Cancel,
    PasteLast,
    CopyLast,
    MeetingNotes,
}

impl Action {
    pub const ALL: [Action; 6] = [
        Action::PushToTalk,
        Action::HandsFree,
        Action::Cancel,
        Action::PasteLast,
        Action::CopyLast,
        Action::MeetingNotes,
    ];
}

/// The combo bound to each action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bindings {
    pub push_to_talk: Combo,
    pub hands_free: Combo,
    pub cancel: Combo,
    pub paste_last: Combo,
    pub copy_last: Combo,
    pub meeting_notes: Combo,
}

impl Bindings {
    /// The defaults table of spec §2.
    pub fn default_for(platform: Platform) -> Self {
        use Key::{Alt, Char, Ctrl, Esc, Fn, Meta, Shift, Space};
        const E: Side = Side::Either;
        match platform {
            Platform::MacOs => Self {
                push_to_talk: Combo::new([Fn]),
                hands_free: Combo::new([Fn, Space]),
                cancel: Combo::new([Esc]),
                paste_last: Combo::new([Ctrl(E), Meta(E), Char('V')]),
                copy_last: Combo::new([Ctrl(E), Meta(E), Char('C')]),
                meeting_notes: Combo::new([Alt(E), Char('M')]),
            },
            Platform::Windows | Platform::Linux => Self {
                push_to_talk: Combo::new([Ctrl(E), Meta(E)]),
                hands_free: Combo::new([Ctrl(E), Meta(E), Space]),
                cancel: Combo::new([Esc]),
                paste_last: Combo::new([Shift(E), Alt(E), Char('Z')]),
                copy_last: Combo::new([Shift(E), Alt(E), Char('X')]),
                meeting_notes: Combo::new([Meta(E), Alt(E), Char('M')]),
            },
        }
    }

    pub fn get(&self, action: Action) -> &Combo {
        match action {
            Action::PushToTalk => &self.push_to_talk,
            Action::HandsFree => &self.hands_free,
            Action::Cancel => &self.cancel,
            Action::PasteLast => &self.paste_last,
            Action::CopyLast => &self.copy_last,
            Action::MeetingNotes => &self.meeting_notes,
        }
    }
}
