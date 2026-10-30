//! The dictation pipeline's swappable stages (ADR-0003). Each engine sits behind one small
//! trait so tiers and settings can pick backends at runtime. Implementations land with their
//! tickets: VAD (06), ASR (07), rule cleanup (08), LLM cleanup (09), local passes (10).

use wf_core::audio::AudioClip;

pub mod rules;

/// Engine failures that the pill's Error state and the fallbacks need to tell apart.
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("model not installed: {0}")]
    ModelMissing(String),
    #[error("timed out")]
    Timeout,
    #[error("{0}")]
    Engine(String),
}

/// A span of speech, as sample indices into the clip it was found in (end exclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeechSegment {
    pub start: usize,
    pub end: usize,
}

/// Voice activity detection (Silero, ticket 06).
pub trait Vad: Send {
    fn speech_segments(&mut self, clip: &AudioClip) -> Result<Vec<SpeechSegment>, PipelineError>;
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

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LlmPrompt {
    pub system: String,
    pub user: String,
    /// `0` lets the engine pick.
    pub max_tokens: u32,
}

/// A local instruct model (llama.cpp, ticket 09) for LLM cleanup, summaries and meeting chat.
pub trait Llm: Send {
    /// Streams tokens to `on_token` as they decode and returns the full completion.
    fn generate(
        &mut self,
        prompt: &LlmPrompt,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<String, PipelineError>;
}
