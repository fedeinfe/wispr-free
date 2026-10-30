//! The dictation pipeline's swappable stages (ADR-0003). Each engine sits behind one small
//! trait so tiers and settings can pick backends at runtime. Implementations land with their
//! tickets: VAD (06), ASR (07), rule cleanup (08), LLM cleanup (09), local passes (10),
//! diarization (16).

use std::time::{Duration, Instant};

use wf_core::audio::AudioClip;

pub mod cleanup;
#[cfg(feature = "diarization")]
pub mod diarization;
#[cfg(feature = "llama")]
pub mod llama;
pub mod llm_host;
pub mod passes;
pub mod rules;
#[cfg(feature = "sherpa")]
pub mod sherpa;
#[cfg(feature = "silero")]
pub mod silero;
pub mod vad;
#[cfg(feature = "whisper")]
pub mod whisper;
pub mod worker;

/// CPU threads for an ASR engine. Speed stops improving much past this (measured on 150 s of
/// speech); ticket 14 checks it on real tiers.
#[cfg(any(feature = "whisper", feature = "sherpa"))]
fn inference_threads() -> usize {
    std::thread::available_parallelism().map_or(4, |cores| cores.get().min(8))
}

/// Engine failures that the pill's Error state and the fallbacks need to tell apart.
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("model not installed: {0}")]
    ModelMissing(String),
    #[error("timed out")]
    Timeout,
    /// The caller stopped the call. Nothing went wrong.
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Engine(String),
}

/// A span of speech, as sample indices into the clip it was found in (end exclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeechSegment {
    pub start: usize,
    pub end: usize,
}

/// Voice activity detection over one recording at a time (Silero, ticket 06). [`vad`] turns the
/// segments into what ASR hears.
pub trait Vad: Send {
    /// Feeds the next samples of the recording and returns the segments that just closed.
    /// A segment closes once the pause after it is long enough to be sure it ended.
    fn accept(&mut self, samples: &[f32]) -> Result<Vec<SpeechSegment>, PipelineError>;

    /// Whether a segment is open. Stays true through the pause that will close it.
    fn is_speaking(&self) -> bool;

    /// Ends the recording, returns the segment still open, and readies for the next recording.
    fn finish(&mut self) -> Result<Vec<SpeechSegment>, PipelineError>;

    /// All the speech in a finished clip.
    fn speech_segments(&mut self, clip: &AudioClip) -> Result<Vec<SpeechSegment>, PipelineError> {
        let mut segments = self.accept(clip.samples())?;
        segments.extend(self.finish()?);
        Ok(segments)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AsrOptions {
    /// BCP-47-ish code ("en", "it"); `None` auto-detects.
    pub language: Option<String>,
    /// Dictionary words and nearby on-screen words, fed as initial prompt or hotwords.
    pub bias_words: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Transcript {
    pub text: String,
    /// The language the engine detected or was told to use.
    pub language: Option<String>,
}

/// Speech recognition (whisper.cpp and sherpa-onnx, ticket 07).
pub trait Asr: Send {
    fn transcribe(
        &mut self,
        clip: &AudioClip,
        options: &AsrOptions,
    ) -> Result<Transcript, PipelineError>;
}

/// One stretch of one voice. `speaker` is the engine's own number and only means something
/// within one [`Diarizer::diarize`] call.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeakerTurn {
    pub start: Duration,
    pub end: Duration,
    pub speaker: u32,
}

/// Who speaks when in one finished recording (sherpa-onnx, ticket 16).
pub trait Diarizer: Send {
    /// The turns, in order of `start`. Turns of different speakers may overlap. A recording
    /// nobody speaks in has none.
    fn diarize(&mut self, clip: &AudioClip) -> Result<Vec<SpeakerTurn>, PipelineError>;
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LlmPrompt {
    pub system: String,
    pub user: String,
    /// Worked (user, assistant) turns placed before `user`. Small models follow them far more
    /// reliably than rules.
    pub examples: Vec<(String, String)>,
    /// `0` lets the engine pick.
    pub max_tokens: u32,
    /// When to give up with [`PipelineError::Timeout`]. `None` waits for as long as it takes.
    pub deadline: Option<Instant>,
}

/// A local instruct model (llama.cpp, ticket 09) for LLM cleanup, summaries and meeting chat.
pub trait Llm: Send {
    /// Streams tokens to `on_token` as they decode and returns the full completion. Engines
    /// check `prompt.deadline` between tokens.
    fn generate(
        &mut self,
        prompt: &LlmPrompt,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<String, PipelineError>;
}
