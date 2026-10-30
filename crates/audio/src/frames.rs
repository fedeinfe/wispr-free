//! Device audio (any rate, any channel count) → pipeline frames (16 kHz mono, 40 ms).

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use wf_core::audio::{FRAME_SAMPLES, SAMPLE_RATE_HZ};

use crate::Error;

/// Downmixes, resamples and slices device audio into pipeline frames of exactly
/// [`FRAME_SAMPLES`] samples. Leftover samples carry over to the next [`push`](Self::push).
pub struct FrameConverter {
    channels: usize,
    /// `None` when the device already runs at the pipeline rate.
    resampler: Option<Fft<f32>>,
    /// Downmixed input waiting for a full resampler chunk.
    mono: Vec<f32>,
    /// Pipeline-rate samples waiting for a full frame.
    pending: Vec<f32>,
    scratch: Vec<f32>,
}

impl FrameConverter {
    pub fn new(input_rate: u32, channels: usize) -> Result<Self, Error> {
        if input_rate == 0 || channels == 0 {
            return Err(Error::UnsupportedFormat {
                rate: input_rate,
                channels,
            });
        }
        let resampler = if input_rate == SAMPLE_RATE_HZ {
            None
        } else {
            // 10 ms input chunks divide evenly for the usual 44.1/48/96 kHz device rates.
            let chunk = (input_rate as usize / 100).max(1);
            let resampler = Fft::new(
                input_rate as usize,
                SAMPLE_RATE_HZ as usize,
                chunk,
                1,
                FixedSync::Input,
            )
            .map_err(|_| Error::UnsupportedFormat {
                rate: input_rate,
                channels,
            })?;
            Some(resampler)
        };
        let scratch = vec![0.0; resampler.as_ref().map_or(0, |r| r.output_frames_max())];
        Ok(Self {
            channels,
            resampler,
            mono: Vec::new(),
            pending: Vec::new(),
            scratch,
        })
    }

    /// Feeds interleaved device samples and returns every frame completed so far.
    pub fn push(&mut self, interleaved: &[f32]) -> Vec<Vec<f32>> {
        let channels = self.channels;
        let downmixed = interleaved
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32);

        match &mut self.resampler {
            None => self.pending.extend(downmixed),
            Some(resampler) => {
                self.mono.extend(downmixed);
                let mut consumed = 0;
                while self.mono.len() - consumed >= resampler.input_frames_next() {
                    let needed = resampler.input_frames_next();
                    let input = &self.mono[consumed..consumed + needed];
                    let input = InterleavedSlice::new(input, 1, needed).expect("mono slice");
                    let capacity = self.scratch.len();
                    let mut output = InterleavedSlice::new_mut(&mut self.scratch, 1, capacity)
                        .expect("mono slice");
                    let (read, written) = resampler
                        .process_into_buffer(&input, &mut output, None)
                        .expect("buffers are sized from the resampler");
                    self.pending.extend_from_slice(&self.scratch[..written]);
                    consumed += read;
                }
                self.mono.drain(..consumed);
            }
        }

        let complete = self.pending.len() / FRAME_SAMPLES * FRAME_SAMPLES;
        let frames = self.pending[..complete]
            .as_chunks::<FRAME_SAMPLES>()
            .0
            .iter()
            .map(|frame| frame.to_vec())
            .collect();
        self.pending.drain(..complete);
        frames
    }
}
