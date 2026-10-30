//! The one audio format the pipeline speaks: 16 kHz mono `f32` (spec §3).

use std::time::Duration;

pub const SAMPLE_RATE_HZ: u32 = 16_000;
pub const FRAME_MS: u32 = 40;
/// Samples in one capture frame.
pub const FRAME_SAMPLES: usize = (SAMPLE_RATE_HZ * FRAME_MS / 1_000) as usize;

/// A run of pipeline-format samples, e.g. one dictation or one meeting chunk.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AudioClip {
    samples: Vec<f32>,
}

impl AudioClip {
    pub fn new(samples: Vec<f32>) -> Self {
        Self { samples }
    }

    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.samples.len() as f64 / f64::from(SAMPLE_RATE_HZ))
    }
}
