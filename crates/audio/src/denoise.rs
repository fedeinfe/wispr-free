//! Optional noise suppression before VAD (spec §3): RNNoise, which only runs at 48 kHz, so
//! pipeline audio goes up to its rate and back.

use nnnoiseless::DenoiseState;
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use wf_core::audio::SAMPLE_RATE_HZ;

use crate::{Error, FrameConverter};

const RNNOISE_RATE_HZ: u32 = 48_000;
/// RNNoise works on 16-bit sample values, not on -1.0..=1.0.
const RNNOISE_SCALE: f32 = 32_768.0;
const RESAMPLER_CHUNK: usize = (SAMPLE_RATE_HZ / 100) as usize;

/// Takes pipeline audio and returns it cleaned, in pipeline frames. Output lags input by a few
/// tens of milliseconds, and a recording's tail shorter than that never comes out. One recording
/// per value: its noise estimate is about that room.
pub struct Denoiser {
    up: Fft<f32>,
    down: FrameConverter,
    state: Box<DenoiseState<'static>>,
    /// Pipeline samples waiting for a full resampler chunk.
    pending: Vec<f32>,
    /// 48 kHz samples waiting for a full RNNoise frame.
    upsampled: Vec<f32>,
    scratch: Vec<f32>,
    /// RNNoise's first frame holds a fade-in artifact.
    first_frame: bool,
}

impl Denoiser {
    pub fn new() -> Result<Self, Error> {
        let unsupported = |_| Error::UnsupportedFormat {
            rate: SAMPLE_RATE_HZ,
            channels: 1,
        };
        let up = Fft::new(
            SAMPLE_RATE_HZ as usize,
            RNNOISE_RATE_HZ as usize,
            RESAMPLER_CHUNK,
            1,
            FixedSync::Input,
        )
        .map_err(unsupported)?;
        let scratch = vec![0.0; up.output_frames_max()];
        Ok(Self {
            up,
            down: FrameConverter::new(RNNOISE_RATE_HZ, 1)?,
            state: DenoiseState::new(),
            pending: Vec::new(),
            upsampled: Vec::new(),
            scratch,
            first_frame: true,
        })
    }

    /// Feeds pipeline samples and returns every pipeline frame finished so far.
    pub fn process(&mut self, samples: &[f32]) -> Vec<Vec<f32>> {
        self.pending.extend_from_slice(samples);
        let mut consumed = 0;
        while self.pending.len() - consumed >= self.up.input_frames_next() {
            let needed = self.up.input_frames_next();
            let input = &self.pending[consumed..consumed + needed];
            let input = InterleavedSlice::new(input, 1, needed).expect("mono slice");
            let capacity = self.scratch.len();
            let mut output =
                InterleavedSlice::new_mut(&mut self.scratch, 1, capacity).expect("mono slice");
            let (read, written) = self
                .up
                .process_into_buffer(&input, &mut output, None)
                .expect("buffers are sized from the resampler");
            self.upsampled.extend_from_slice(&self.scratch[..written]);
            consumed += read;
        }
        self.pending.drain(..consumed);

        let mut cleaned = Vec::new();
        let frames = self.upsampled.len() / DenoiseState::FRAME_SIZE;
        let mut output = [0.0; DenoiseState::FRAME_SIZE];
        for frame in self.upsampled.chunks_exact(DenoiseState::FRAME_SIZE) {
            let input: Vec<f32> = frame.iter().map(|s| s * RNNOISE_SCALE).collect();
            self.state.process_frame(&mut output, &input);
            if !std::mem::take(&mut self.first_frame) {
                cleaned.extend(output.iter().map(|s| s / RNNOISE_SCALE));
            }
        }
        self.upsampled.drain(..frames * DenoiseState::FRAME_SIZE);
        self.down.push(&cleaned)
    }
}
