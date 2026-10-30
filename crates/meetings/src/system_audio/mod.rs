//! System output as a capture source (spec §4, §7). Each platform implements
//! [`wf_audio::AudioBackend`], so a system stream gets the same resampling, framing and
//! reconnecting as a microphone: see [`wf_audio::Capture`].
//!
//! A backend offers one device whose id names the current output device. When the user switches
//! output the id changes, and `Capture`'s polling reopens the stream on the new one.

#[cfg(not(target_os = "macos"))]
mod loopback;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod screen_capture;

#[cfg(not(target_os = "macos"))]
pub use loopback::SystemAudioBackend;
pub use macos::SystemAudioBackend;
#[cfg(target_os = "macos")]
pub(crate) use macos::{object_list, object_property, object_string};
