//! whisper.cpp and llama.cpp each bring their own ggml, and linked wrongly they abort or fail to
//! link (ADR-0003). This runs real inference through every engine in one process, so a change to
//! how they link fails here. Skipped by default:
//!
//! `WF_WHISPER_MODEL=ggml-small-q8_0.bin WF_PARAKEET_DIR=parakeet-tdt-0.6b-v3-int8 WF_LLM_MODEL=model.gguf cargo test -p wf-pipeline --release --features whisper,sherpa,llama --test engines_together -- --ignored`
#![cfg(all(feature = "whisper", feature = "sherpa", feature = "llama"))]

mod support;

use std::path::Path;

use support::fixture;
use wf_pipeline::llama::{LlamaConfig, LlamaEngine};
use wf_pipeline::sherpa::SherpaAsr;
use wf_pipeline::whisper::WhisperAsr;
use wf_pipeline::{Asr, AsrOptions, Llm, LlmPrompt};

fn path(variable: &str) -> String {
    std::env::var(variable).unwrap_or_else(|_| panic!("set {variable}"))
}

fn assert_hears(asr: &mut dyn Asr) {
    let clip = fixture("speech_en");
    let text = asr.transcribe(&clip, &AsrOptions::default()).unwrap().text;
    assert!(text.to_lowercase().contains("translated by"), "{text}");
}

fn assert_answers(llm: &mut dyn Llm) {
    let prompt = LlmPrompt {
        user: "What is the capital of France? Answer with one word.".into(),
        max_tokens: 16,
        ..LlmPrompt::default()
    };
    let reply = llm.generate(&prompt, &mut |_| {}).unwrap();
    assert!(reply.contains("Paris"), "{reply}");
}

#[test]
#[ignore = "needs models"]
fn every_engine_runs_in_one_process_in_any_order() {
    let mut whisper = WhisperAsr::new(Path::new(&path("WF_WHISPER_MODEL"))).unwrap();
    let mut parakeet = SherpaAsr::new(Path::new(&path("WF_PARAKEET_DIR"))).unwrap();
    let mut llama = LlamaEngine::load(&LlamaConfig::new(path("WF_LLM_MODEL"))).unwrap();

    assert_hears(&mut whisper);
    assert_answers(&mut llama);
    assert_hears(&mut parakeet);
    assert_answers(&mut llama);
    assert_hears(&mut whisper);
}
