//! Engines are swapped at runtime by tier and settings, and run off the UI thread,
//! so each stage must work as a boxed trait object sent to a worker.

use std::thread;

use wf_core::audio::AudioClip;
use wf_pipeline::{Asr, AsrOptions, Llm, LlmPrompt, PipelineError, SpeechSegment, Transcript, Vad};

struct EverythingIsSpeech;

impl Vad for EverythingIsSpeech {
    fn speech_segments(&mut self, clip: &AudioClip) -> Result<Vec<SpeechSegment>, PipelineError> {
        Ok(vec![SpeechSegment {
            start: 0,
            end: clip.samples().len(),
        }])
    }
}

struct CannedAsr(&'static str);

impl Asr for CannedAsr {
    fn transcribe(
        &mut self,
        _clip: &AudioClip,
        options: &AsrOptions,
    ) -> Result<Transcript, PipelineError> {
        Ok(Transcript {
            text: self.0.to_owned(),
            language: options.language.clone(),
        })
    }
}

struct EchoLlm;

impl Llm for EchoLlm {
    fn generate(
        &mut self,
        prompt: &LlmPrompt,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<String, PipelineError> {
        for word in prompt.user.split_inclusive(' ') {
            on_token(word);
        }
        Ok(prompt.user.clone())
    }
}

struct NoModel;

impl Asr for NoModel {
    fn transcribe(
        &mut self,
        _clip: &AudioClip,
        _options: &AsrOptions,
    ) -> Result<Transcript, PipelineError> {
        Err(PipelineError::ModelMissing("whisper-large-v3-turbo".into()))
    }
}

#[test]
fn boxed_engines_run_a_dictation_on_a_worker_thread() {
    let mut vad: Box<dyn Vad> = Box::new(EverythingIsSpeech);
    let mut asr: Box<dyn Asr> = Box::new(CannedAsr("ship it"));
    let mut llm: Box<dyn Llm> = Box::new(EchoLlm);

    let (segments, transcript, streamed, cleaned) = thread::spawn(move || {
        let clip = AudioClip::new(vec![0.0; 1_600]);
        let segments = vad.speech_segments(&clip).unwrap();
        let options = AsrOptions {
            language: Some("en".into()),
            ..AsrOptions::default()
        };
        let transcript = asr.transcribe(&clip, &options).unwrap();
        let mut streamed = Vec::new();
        let prompt = LlmPrompt {
            user: transcript.text.clone(),
            ..LlmPrompt::default()
        };
        let cleaned = llm
            .generate(&prompt, &mut |token| streamed.push(token.to_owned()))
            .unwrap();
        (segments, transcript, streamed, cleaned)
    })
    .join()
    .unwrap();

    assert_eq!(
        segments,
        vec![SpeechSegment {
            start: 0,
            end: 1_600
        }]
    );
    assert_eq!(
        transcript,
        Transcript {
            text: "ship it".into(),
            language: Some("en".into())
        }
    );
    assert_eq!(streamed, vec!["ship ", "it"]);
    assert_eq!(cleaned, "ship it");
}

#[test]
fn a_missing_model_names_the_model_in_the_error() {
    let mut asr: Box<dyn Asr> = Box::new(NoModel);

    let error = asr
        .transcribe(&AudioClip::default(), &AsrOptions::default())
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "model not installed: whisper-large-v3-turbo"
    );
}
