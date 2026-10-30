//! Global key press/release streams (spec §7): a CGEventTap plus IOHID for Fn on macOS, a
//! low-level hook on Windows, evdev on Linux. Not register-a-hotkey APIs: push-to-talk needs the
//! release too. `examples/keys.rs` prints the stream through the dictation state machine.

pub mod keymap;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as backend;
#[cfg(target_os = "macos")]
use macos as backend;
#[cfg(target_os = "windows")]
use windows as backend;

use wf_core::shortcut::KeyEvent;

/// What a backend reports. Events from our own synthetic keystrokes (insertion) are filtered out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySignal {
    Key(KeyEvent),
    /// Events may have been missed, so nothing is known about which keys are down.
    Lost,
}

#[derive(Debug, thiserror::Error)]
pub enum KeysError {
    /// The OS refuses access; the message says which permission is missing.
    #[error("{0}")]
    Permission(&'static str),
    #[error("could not start key capture: {0}")]
    Unavailable(String),
}

/// Called on the backend's thread for each signal. Returning `true` keeps a key event from the
/// focused app where the OS allows it (not on Linux). Keep it fast: macOS disables slow event taps.
pub type Callback = Box<dyn FnMut(KeySignal) -> bool + Send>;

/// Key capture is running until this is dropped.
pub struct KeyListener {
    backend: backend::Listener,
}

impl KeyListener {
    /// Whether returning `true` from the callback really hides the event. Without it, the Space of
    /// Fn+Space reaches the focused app.
    pub fn can_swallow(&self) -> bool {
        self.backend.can_swallow()
    }
}

/// Starts capturing keys globally.
pub fn listen(
    on_signal: impl FnMut(KeySignal) -> bool + Send + 'static,
) -> Result<KeyListener, KeysError> {
    Ok(KeyListener {
        backend: backend::Listener::start(Box::new(on_signal))?,
    })
}
