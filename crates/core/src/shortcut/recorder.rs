//! The Settings shortcut recorder's capture loop (spec §2): the combo is what was held at its
//! fullest, committed once every key is up, and Esc alone aborts.

use super::{Action, Bindings, Combo, Key, KeyEvent, Platform, ShortcutError, Side, validate};

/// What a key event did to the recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecorderStep {
    /// Still recording, with these keys held.
    Recording(Combo),
    /// The user aborted. The binding stays as it was.
    Aborted,
    /// Every key is up and the combo is valid. The caller stores it.
    Committed(Combo),
    /// Every key is up and the combo was refused. Recording continues from an empty state.
    Rejected(Combo, ShortcutError),
}

/// One recording session for one action.
#[derive(Debug, Clone)]
pub struct ShortcutRecorder {
    action: Action,
    bindings: Bindings,
    platform: Platform,
    held: Vec<Key>,
    fullest: Vec<Key>,
}

impl ShortcutRecorder {
    pub fn new(action: Action, bindings: Bindings, platform: Platform) -> Self {
        Self {
            action,
            bindings,
            platform,
            held: Vec::new(),
            fullest: Vec::new(),
        }
    }

    pub fn on_key(&mut self, event: KeyEvent) -> RecorderStep {
        if event.pressed {
            self.on_press(event.key)
        } else {
            self.on_release(event.key)
        }
    }

    fn on_press(&mut self, key: Key) -> RecorderStep {
        if key == Key::Esc && self.action != Action::Cancel && self.held.is_empty() {
            return RecorderStep::Aborted;
        }
        if !self.held.contains(&key) {
            self.held.push(key);
        }
        if self.held.len() >= self.fullest.len() {
            self.fullest.clone_from(&self.held);
        }
        RecorderStep::Recording(Combo::new(self.held.clone()))
    }

    /// A release of a key that was not pressed here, such as the one that opened the recorder, is
    /// ignored.
    fn on_release(&mut self, key: Key) -> RecorderStep {
        let was_held = self.held.contains(&key);
        self.held.retain(|&held| held != key);
        if !was_held || !self.held.is_empty() {
            return RecorderStep::Recording(Combo::new(self.held.clone()));
        }
        let combo = normalized(&std::mem::take(&mut self.fullest));
        match validate(self.action, &combo, &self.bindings, self.platform) {
            Ok(()) => RecorderStep::Committed(combo),
            Err(error) => RecorderStep::Rejected(combo, error),
        }
    }
}

/// Modifiers first in a fixed order. With a regular key the combo is "Ctrl+J", either side; a
/// combo of modifiers alone keeps its sides so that Right Cmd stays bindable.
fn normalized(keys: &[Key]) -> Combo {
    let has_regular_key = keys.iter().any(|key| !key.is_modifier());
    let mut keys: Vec<Key> = keys
        .iter()
        .map(|&key| if has_regular_key { unsided(key) } else { key })
        .collect();
    keys.sort_by_key(|&key| modifier_rank(key));
    Combo::new(keys)
}

fn unsided(key: Key) -> Key {
    match key {
        Key::Ctrl(_) => Key::Ctrl(Side::Either),
        Key::Shift(_) => Key::Shift(Side::Either),
        Key::Alt(_) => Key::Alt(Side::Either),
        Key::Meta(_) => Key::Meta(Side::Either),
        key => key,
    }
}

/// The order of the spec's defaults table: Ctrl+Cmd, Shift+Alt, Win+Alt.
fn modifier_rank(key: Key) -> u8 {
    match key {
        Key::Fn => 0,
        Key::Ctrl(_) => 1,
        Key::Shift(_) => 2,
        Key::Meta(_) => 3,
        Key::Alt(_) => 4,
        _ => 5,
    }
}

/// Puts `action` back on its default for `platform`, unless another action holds that combo now.
pub fn reset_to_default(
    action: Action,
    bindings: &mut Bindings,
    platform: Platform,
) -> Result<(), ShortcutError> {
    let default = Bindings::default_for(platform).get(action).clone();
    validate(action, &default, bindings, platform)?;
    bindings.set(action, default);
    Ok(())
}
