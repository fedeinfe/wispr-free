//! Microphone capture into the pipeline format (`wf_core::audio`): device selection and
//! hot-plug, resampling, the ~300 ms pre-roll ring buffer and the level meter (ticket 02).
//! Optional RNNoise denoise also lives here, as do the pill's synthesized [`sounds`] (ticket 05).
//!
//! [`Capture`] runs a thread that keeps the best-ranked mic open and emits 40 ms frames. The
//! consumer feeds those frames to a [`Recorder`] (dictation clips with pre-roll) and a
//! [`LevelMeter`] (pill waveform).

mod capture;
mod cpal_backend;
mod denoise;
mod devices;
mod frames;
mod level;
mod recorder;
pub mod sounds;

pub use capture::{AudioBackend, Capture, CaptureConfig, CaptureEvent, StreamFormat, StreamSink};
pub use cpal_backend::{CpalBackend, open_stream};
pub use denoise::Denoiser;
pub use devices::{InputDevice, rank_devices};
pub use frames::FrameConverter;
pub use level::{LEVEL_WINDOW_MS, LevelMeter};
pub use recorder::{PRE_ROLL_MS, Recorder};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unsupported input format: {rate} Hz, {channels} channels")]
    UnsupportedFormat { rate: u32, channels: usize },
    #[error("input device {0} is not available")]
    DeviceUnavailable(String),
    #[error("audio backend error: {0}")]
    Backend(String),
}
