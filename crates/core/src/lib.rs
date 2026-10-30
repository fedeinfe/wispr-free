//! Engine-free logic shared by every other crate: the audio format the pipeline speaks, the
//! shortcut model, the dictation state machine, the dictation history, and later settings.

pub mod audio;
pub mod dictation;
pub mod history;
pub mod shortcut;
