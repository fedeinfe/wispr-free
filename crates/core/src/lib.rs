//! Pure, engine-free logic shared by every other crate: the audio format the pipeline speaks,
//! the shortcut model, the dictation state machine and the settings.

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!("wfsettings");

pub mod audio;
pub mod dictation;
pub mod pill;
pub mod settings;
pub mod shortcut;
