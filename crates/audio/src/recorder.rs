//! Turns the continuous frame stream into dictation clips that begin slightly before key-down.

use std::collections::VecDeque;

use wf_core::audio::{AudioClip, SAMPLE_RATE_HZ};

/// How much audio before [`Recorder::start`] a clip keeps, so the first syllable is never clipped.
pub const PRE_ROLL_MS: u32 = 300;
const PRE_ROLL_SAMPLES: usize = (SAMPLE_RATE_HZ * PRE_ROLL_MS / 1_000) as usize;

/// Feed it every capture frame. While idle it remembers the last [`PRE_ROLL_MS`]; between
/// [`start`](Self::start) and [`stop`](Self::stop) it records.
#[derive(Debug, Default)]
pub struct Recorder {
    pre_roll: VecDeque<f32>,
    clip: Option<Vec<f32>>,
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, samples: &[f32]) {
        match &mut self.clip {
            Some(clip) => clip.extend_from_slice(samples),
            None => {
                self.pre_roll.extend(samples);
                let excess = self.pre_roll.len().saturating_sub(PRE_ROLL_SAMPLES);
                self.pre_roll.drain(..excess);
            }
        }
    }

    /// Starts a clip seeded with the pre-roll. Calling it while recording keeps the current clip.
    pub fn start(&mut self) {
        if self.clip.is_none() {
            self.clip = Some(self.pre_roll.drain(..).collect());
        }
    }

    /// Ends the clip and returns it (empty if not recording).
    pub fn stop(&mut self) -> AudioClip {
        AudioClip::new(self.clip.take().unwrap_or_default())
    }

    pub fn is_recording(&self) -> bool {
        self.clip.is_some()
    }
}
