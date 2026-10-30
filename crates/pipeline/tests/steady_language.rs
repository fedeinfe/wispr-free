//! Holding a recording to one language, whatever the chunks would guess by themselves.

use std::time::Duration;

use wf_core::audio::{AudioClip, SAMPLE_RATE_HZ};
use wf_pipeline::language::{SETTLING_SPEECH, SteadyLanguage};
use wf_pipeline::{Asr, AsrOptions, PipelineError, Transcript};

/// Reports what it was told, or `detects` if it wasn't, and keeps what it was asked.
struct Scripted {
    detects: Option<&'static str>,
    asked: Vec<Option<String>>,
}

impl Scripted {
    fn detecting(language: Option<&'static str>) -> Self {
        Self {
            detects: language,
            asked: Vec::new(),
        }
    }
}

impl Asr for Scripted {
    fn transcribe(
        &mut self,
        _clip: &AudioClip,
        options: &AsrOptions,
    ) -> Result<Transcript, PipelineError> {
        self.asked.push(options.language.clone());
        let language = options.language.clone();
        Ok(Transcript {
            text: "words".to_owned(),
            language: language.or(self.detects.map(str::to_owned)),
        })
    }
}

fn clip(length: Duration) -> AudioClip {
    let samples = length.as_secs_f64() * f64::from(SAMPLE_RATE_HZ);
    AudioClip::new(vec![0.1; samples as usize])
}

fn short() -> AudioClip {
    clip(Duration::from_secs(2))
}

fn long() -> AudioClip {
    clip(SETTLING_SPEECH)
}

#[test]
fn short_chunks_use_the_language_of_the_speech_that_settled_it() {
    let mut asr = Scripted::detecting(Some("it"));
    let mut language = SteadyLanguage::default();
    let options = AsrOptions::default();

    language.settle(&mut asr, &long(), &options).unwrap();
    asr.detects = Some("pl");
    let heard = language.transcribe(&mut asr, &short(), &options).unwrap();

    assert_eq!(heard.language.as_deref(), Some("it"));
    assert_eq!(asr.asked, [None, Some("it".to_owned())]);
}

#[test]
fn a_chunk_long_enough_to_judge_changes_the_language() {
    let mut asr = Scripted::detecting(Some("it"));
    let mut language = SteadyLanguage::default();
    let options = AsrOptions::default();
    language.settle(&mut asr, &long(), &options).unwrap();

    asr.detects = Some("en");
    language.transcribe(&mut asr, &long(), &options).unwrap();
    let heard = language.transcribe(&mut asr, &short(), &options).unwrap();

    assert_eq!(heard.language.as_deref(), Some("en"));
}

#[test]
fn a_short_chunk_never_decides() {
    let mut asr = Scripted::detecting(Some("pl"));
    let mut language = SteadyLanguage::default();
    let options = AsrOptions::default();

    language.transcribe(&mut asr, &short(), &options).unwrap();
    language.transcribe(&mut asr, &short(), &options).unwrap();

    assert_eq!(asr.asked, [None, None]);
}

#[test]
fn a_language_the_user_chose_is_never_detected_or_overruled() {
    let mut asr = Scripted::detecting(Some("pl"));
    let mut language = SteadyLanguage::default();
    let options = AsrOptions {
        language: Some("de".to_owned()),
        ..AsrOptions::default()
    };

    language.settle(&mut asr, &long(), &options).unwrap();
    assert!(asr.asked.is_empty(), "nothing to decide");

    language.transcribe(&mut asr, &short(), &options).unwrap();
    language.transcribe(&mut asr, &long(), &options).unwrap();
    assert_eq!(asr.asked, [Some("de".to_owned()), Some("de".to_owned())]);
}

#[test]
fn an_engine_that_reports_no_language_is_left_to_itself() {
    let mut asr = Scripted::detecting(None);
    let mut language = SteadyLanguage::default();
    let options = AsrOptions::default();

    language.settle(&mut asr, &long(), &options).unwrap();
    language.transcribe(&mut asr, &short(), &options).unwrap();

    assert_eq!(asr.asked, [None, None]);
}
