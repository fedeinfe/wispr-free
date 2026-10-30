//! Pure, engine-free logic shared by every other crate: the audio format the pipeline speaks,
//! the shortcut model and the dictation state machine, and later settings and history (ticket 10).

pub mod audio;
pub mod dictation;
pub mod shortcut;
