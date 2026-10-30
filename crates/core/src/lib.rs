//! Pure, engine-free logic shared by every other crate: the audio format the pipeline speaks,
//! the shortcut model, the dictation state machine and the settings, and later history (ticket 10).

pub mod audio;
pub mod dictation;
pub mod settings;
pub mod shortcut;
