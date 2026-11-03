//! Keys a backend reported down and not yet up. A release the OS never delivers, such as one that
//! happens after secure input began (a password field, the lock screen), would leave the key down
//! in the dictation state machine for good, and no shortcut would match again. So each press first
//! asks the OS which of the earlier keys are really still down, and reports the others released.
//! Pure, so it is tested on every OS.

use wf_core::shortcut::{Key, KeyEvent};

#[derive(Debug, Default)]
pub struct Held(Vec<Key>);

impl Held {
    /// Notes `event` and returns the releases to report before it: earlier keys that `is_down`
    /// says are up. `is_down` answers `None` when it can't tell, which keeps the key.
    pub fn on_event(
        &mut self,
        event: KeyEvent,
        is_down: impl Fn(Key) -> Option<bool>,
    ) -> Vec<KeyEvent> {
        if !event.pressed {
            self.0.retain(|&key| key != event.key);
            return Vec::new();
        }
        let mut missed = Vec::new();
        self.0.retain(|&key| {
            let up = key != event.key && is_down(key) == Some(false);
            if up {
                missed.push(KeyEvent::release(key));
            }
            !up
        });
        if !self.0.contains(&event.key) {
            self.0.push(event.key);
        }
        missed
    }

    /// Events were missed; the state machine forgets every key too.
    pub fn clear(&mut self) {
        self.0.clear();
    }
}
