//! Silero VAD through sherpa-onnx (ADR-0003). Behind the `silero` feature because sherpa-onnx
//! fetches prebuilt native libraries at build time.

use std::path::Path;

use sherpa_onnx::{SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};
use wf_core::audio::SAMPLE_RATE_HZ;

use crate::{PipelineError, SpeechSegment, Vad};

/// Silero v5's window at 16 kHz.
const WINDOW_SAMPLES: usize = 512;
/// sherpa's defaults are zero, so every threshold is set here.
const SPEECH_THRESHOLD: f32 = 0.5;
/// What counts as a pause. Long enough that closing a chunk lands between phrases.
const MIN_SILENCE_SECONDS: f32 = 0.5;
const MIN_SPEECH_SECONDS: f32 = 0.25;
/// Effectively off: where to split long speech is [`crate::vad`]'s decision, not sherpa's.
const MAX_SPEECH_SECONDS: f32 = 3_600.0;
/// sherpa's audio buffer. It grows without loss past this, but logs to stderr when it does.
const BUFFER_SECONDS: f32 = 60.0;

pub struct SileroVad {
    detector: VoiceActivityDetector,
}

impl SileroVad {
    pub fn new(model: &Path) -> Result<Self, PipelineError> {
        if !model.is_file() {
            return Err(PipelineError::ModelMissing(model.display().to_string()));
        }
        let config = VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: Some(model.to_string_lossy().into_owned()),
                threshold: SPEECH_THRESHOLD,
                min_silence_duration: MIN_SILENCE_SECONDS,
                min_speech_duration: MIN_SPEECH_SECONDS,
                window_size: WINDOW_SAMPLES as i32,
                max_speech_duration: MAX_SPEECH_SECONDS,
            },
            sample_rate: SAMPLE_RATE_HZ as i32,
            num_threads: 1,
            provider: Some("cpu".into()),
            ..VadModelConfig::default()
        };
        let detector = VoiceActivityDetector::create(&config, BUFFER_SECONDS)
            .ok_or_else(|| PipelineError::Engine("Silero VAD failed to load".into()))?;
        Ok(Self { detector })
    }

    fn take_closed(&self) -> Vec<SpeechSegment> {
        let mut closed = Vec::new();
        while let Some(segment) = self.detector.front() {
            let start = segment.start() as usize;
            closed.push(SpeechSegment {
                start,
                end: start + segment.n() as usize,
            });
            self.detector.pop();
        }
        closed
    }
}

impl Vad for SileroVad {
    fn accept(&mut self, samples: &[f32]) -> Result<Vec<SpeechSegment>, PipelineError> {
        // One window per call: sherpa treats a longer call as speech if any window in it is.
        for window in samples.chunks(WINDOW_SAMPLES) {
            self.detector.accept_waveform(window);
        }
        Ok(self.take_closed())
    }

    fn is_speaking(&self) -> bool {
        self.detector.detected()
    }

    fn finish(&mut self) -> Result<Vec<SpeechSegment>, PipelineError> {
        self.detector.flush();
        let closed = self.take_closed();
        self.detector.reset();
        Ok(closed)
    }
}
