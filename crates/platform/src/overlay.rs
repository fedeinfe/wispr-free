//! The pill window's native behaviour (spec §1, §7, ADR-0002): non-activating, always on top, on
//! every Space, plus the cursor, screen and Esc state the pill polls because its window never
//! gets keyboard focus. Everything here must be called from the UI thread.

use raw_window_handle::{HandleError, HasWindowHandle};
use wf_core::pill::dock::{Display, Point};

#[cfg(target_os = "linux")]
#[path = "overlay/x11.rs"]
mod backend;

#[cfg(target_os = "macos")]
#[path = "overlay/macos.rs"]
mod backend;

#[cfg(target_os = "windows")]
#[path = "overlay/windows.rs"]
mod backend;

#[derive(Debug, thiserror::Error)]
pub enum OverlayError {
    #[error("no native window handle: {0}")]
    Handle(#[from] HandleError),
    #[error("not a window this backend can manage")]
    UnsupportedWindow,
    #[error("the overlay must be set up on the UI thread")]
    NotOnUiThread,
    #[error("the OS refused the window change: {0}")]
    Os(String),
}

/// Turns a freshly created, already visible window into the overlay.
pub fn apply(window: &impl HasWindowHandle) -> Result<(), OverlayError> {
    backend::apply(window)
}

/// Restores the overlay's stacking and styles, for what the OS or winit does not preserve. Call it
/// after changing winit's hit-testing, and now and then.
pub fn raise(window: &impl HasWindowHandle) -> Result<(), OverlayError> {
    backend::raise(window)
}

/// Every screen, in the top-left-origin desktop units of `wf_core::pill::dock`. `window_scale`
/// is the Slint window's scale factor, for backends with no per-screen scale of their own.
pub fn displays(window_scale: f32) -> Vec<Display> {
    backend::displays(window_scale)
}

/// The mouse position, wherever it is. `None` where the OS does not reveal it.
pub fn cursor_position() -> Option<Point> {
    backend::cursor_position()
}

/// Whether Esc is down right now, whichever app has the keyboard.
pub fn escape_pressed() -> bool {
    backend::escape_pressed()
}
