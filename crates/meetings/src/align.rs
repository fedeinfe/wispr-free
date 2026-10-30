//! Pairs up the two capture streams. They run on different clocks and either can stall or stop,
//! so the slower side is filled with silence rather than letting the file fall behind real time.

use std::collections::VecDeque;

use wf_core::audio::FRAME_SAMPLES;

/// One side may run this far ahead before the other is padded. Long enough to ride out normal
/// jitter, and it adds to what a crash can lose.
const MAX_SKEW_FRAMES: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// The microphone.
    You,
    /// System audio.
    Others,
}

/// 40 ms of both channels on the shared timeline.
#[derive(Debug, Clone, PartialEq)]
pub struct StereoFrame {
    pub you: Vec<f32>,
    pub others: Vec<f32>,
}

#[derive(Debug, Default)]
pub struct StereoAligner {
    you: VecDeque<f32>,
    others: VecDeque<f32>,
}

impl StereoAligner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds samples of one channel and returns every frame now ready to be written.
    pub fn push(&mut self, channel: Channel, samples: Vec<f32>) -> Vec<StereoFrame> {
        match channel {
            Channel::You => self.you.extend(samples),
            Channel::Others => self.others.extend(samples),
        }
        let mut frames = Vec::new();
        while self.you.len().min(self.others.len()) >= FRAME_SAMPLES {
            frames.push(self.take_frame());
        }
        let limit = (MAX_SKEW_FRAMES + 1) * FRAME_SAMPLES;
        while self.you.len().max(self.others.len()) >= limit {
            frames.push(self.take_frame());
        }
        frames
    }

    /// Everything still queued, padded to whole frames. Use at the end of a recording.
    pub fn drain(&mut self) -> Vec<StereoFrame> {
        let mut frames = Vec::new();
        while !self.you.is_empty() || !self.others.is_empty() {
            frames.push(self.take_frame());
        }
        frames
    }

    fn take_frame(&mut self) -> StereoFrame {
        StereoFrame {
            you: take_padded(&mut self.you),
            others: take_padded(&mut self.others),
        }
    }
}

fn take_padded(queue: &mut VecDeque<f32>) -> Vec<f32> {
    let available = queue.len().min(FRAME_SAMPLES);
    let mut frame: Vec<f32> = queue.drain(..available).collect();
    frame.resize(FRAME_SAMPLES, 0.0);
    frame
}
