//! Everything that differs per OS, behind one cross-platform API per capability (spec §7):
//! global key press/release streams (ticket 03), insertion and focused-text context (04), launch
//! at login and handing links to the OS (12), permission state (19), and quieting media while
//! dictating (20).
//! Each capability gets a module with `#[cfg(target_os = ...)]` backends inside it.

pub mod autostart;
pub mod context;
pub mod desktop;
pub mod insert;
pub mod keys;
pub mod media;
pub mod permissions;
