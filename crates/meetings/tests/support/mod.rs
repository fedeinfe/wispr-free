//! Fake engines for the meeting tests.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use wf_core::audio::AudioClip;
use wf_pipeline::{
    Asr, AsrOptions, Diarizer, Llm, LlmPrompt, PipelineError, SpeakerTurn, SpeechSegment,
    Transcript, Vad,
};

/// The audible parts of the recording. Any level above this is speech.
const LOUD: f32 = 0.05;
/// A pause this long closes a segment, like Silero's.
const PAUSE_SAMPLES: usize = 8_000;

/// A level detector with Silero's habits: closes a segment after a pause, `finish` flushes.
#[derive(Default)]
pub struct LevelVad {
    position: usize,
    speech_start: Option<usize>,
    last_loud: usize,
}

impl Vad for LevelVad {
    fn accept(&mut self, samples: &[f32]) -> Result<Vec<SpeechSegment>, PipelineError> {
        let mut closed = Vec::new();
        for &sample in samples {
            if sample.abs() > LOUD {
                self.last_loud = self.position;
                self.speech_start.get_or_insert(self.position);
            } else if let Some(start) = self.speech_start
                && self.position - self.last_loud >= PAUSE_SAMPLES
            {
                closed.push(SpeechSegment {
                    start,
                    end: self.last_loud + 1,
                });
                self.speech_start = None;
            }
            self.position += 1;
        }
        Ok(closed)
    }

    fn is_speaking(&self) -> bool {
        self.speech_start.is_some()
    }

    fn finish(&mut self) -> Result<Vec<SpeechSegment>, PipelineError> {
        let open = self.speech_start.take().map(|start| SpeechSegment {
            start,
            end: self.last_loud + 1,
        });
        *self = Self::default();
        Ok(open.into_iter().collect())
    }
}

/// Says back how long each chunk is, so a test can tell which audio was heard.
#[derive(Default)]
pub struct LengthAsr;

impl Asr for LengthAsr {
    fn transcribe(
        &mut self,
        clip: &AudioClip,
        _options: &AsrOptions,
    ) -> Result<Transcript, PipelineError> {
        Ok(Transcript {
            text: format!("heard {} ms", clip.duration().as_millis()),
            language: None,
        })
    }
}

/// Fails every time.
pub struct BrokenAsr;

impl Asr for BrokenAsr {
    fn transcribe(
        &mut self,
        _clip: &AudioClip,
        _options: &AsrOptions,
    ) -> Result<Transcript, PipelineError> {
        Err(PipelineError::Engine("ASR is broken".into()))
    }
}

/// Answers with fixed turns.
pub struct FixedDiarizer(pub Vec<SpeakerTurn>);

impl Diarizer for FixedDiarizer {
    fn diarize(&mut self, _clip: &AudioClip) -> Result<Vec<SpeakerTurn>, PipelineError> {
        Ok(self.0.clone())
    }
}

pub struct BrokenDiarizer;

impl Diarizer for BrokenDiarizer {
    fn diarize(&mut self, _clip: &AudioClip) -> Result<Vec<SpeakerTurn>, PipelineError> {
        Err(PipelineError::Engine("diarization is broken".into()))
    }
}

type Reply = Box<dyn FnMut(&LlmPrompt) -> Result<String, PipelineError> + Send>;

/// Replies through a function and keeps every prompt it was given.
#[derive(Clone)]
pub struct ScriptedLlm {
    reply: Arc<Mutex<Reply>>,
    pub prompts: Arc<Mutex<Vec<LlmPrompt>>>,
}

impl ScriptedLlm {
    pub fn new(
        reply: impl FnMut(&LlmPrompt) -> Result<String, PipelineError> + Send + 'static,
    ) -> Self {
        Self {
            reply: Arc::new(Mutex::new(Box::new(reply))),
            prompts: Arc::default(),
        }
    }

    pub fn prompts(&self) -> Vec<LlmPrompt> {
        self.prompts.lock().unwrap().clone()
    }
}

impl Llm for ScriptedLlm {
    fn generate(
        &mut self,
        prompt: &LlmPrompt,
        _on_token: &mut dyn FnMut(&str),
    ) -> Result<String, PipelineError> {
        self.prompts.lock().unwrap().push(prompt.clone());
        (self.reply.lock().unwrap())(prompt)
    }
}
