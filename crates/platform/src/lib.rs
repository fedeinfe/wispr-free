//! Everything that differs per OS, behind one cross-platform API per capability (spec §7):
//! global key press/release streams (ticket 03), insertion and focused-text context (04),
//! the pill window's non-activating, click-through, all-Spaces behaviour (05), and launch at
//! login, Dock visibility and handing links to the OS (12).
//! Each capability gets a module with `#[cfg(target_os = ...)]` backends inside it.

pub mod autostart;
pub mod desktop;
pub mod keys;
