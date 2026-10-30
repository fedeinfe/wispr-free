//! The real llama.cpp engine against a real model. Skipped by default:
//! `WF_LLM_MODEL=model.gguf cargo test -p wf-pipeline --features llama --release -- --ignored`

#![cfg(feature = "llama")]

use std::time::{Duration, Instant};

use wf_pipeline::cleanup::{CleanupRequest, DEFAULT_TIMEOUT, Fallback, llm_cleanup};
use wf_pipeline::llama::{LlamaConfig, LlamaEngine};
use wf_pipeline::llm_host::{KeepLoaded, LlmHost};
use wf_pipeline::rules::rule_cleanup;
use wf_pipeline::{Llm, LlmPrompt, PipelineError};

fn model() -> String {
    std::env::var("WF_LLM_MODEL").expect("set WF_LLM_MODEL to a GGUF file")
}

fn host(keep: KeepLoaded) -> LlmHost {
    let config = LlamaConfig::new(model());
    LlmHost::spawn(
        move || Ok(Box::new(LlamaEngine::load(&config)?) as Box<dyn Llm>),
        keep,
    )
}

fn resident_kilobytes() -> u64 {
    let pid = std::process::id().to_string();
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap()
}

/// Sentences that differ, so the model has to copy them all instead of collapsing a repeat.
fn dictation(sentences: usize) -> String {
    let (who, what) = (
        ["Anna", "Marco", "Julia", "Tom", "Sara", "Paolo", "Nina"],
        [
            "the budget",
            "the roadmap",
            "the invoice",
            "the draft",
            "the contract",
        ],
    );
    (0..sentences)
        .map(|i| {
            format!(
                "{} will review {} in room {} before the call on day {}.",
                who[i % who.len()],
                what[i % what.len()],
                i + 1,
                i % 28 + 1,
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn a_missing_file_names_the_model() {
    let error = LlamaEngine::load(&LlamaConfig::new("/nowhere/model.gguf"))
        .err()
        .unwrap();

    assert!(matches!(error, PipelineError::ModelMissing(_)), "{error}");
}

#[test]
#[ignore = "needs a model"]
fn spec_examples_come_out_clean() {
    let mut llm = host(KeepLoaded::Always);
    for (asr, expected) in [
        (
            "I like the red one, oh no, I meant the blue one",
            "I like the blue one.",
        ),
        (
            "um so I I think we should uh ship it",
            "So I think we should ship it.",
        ),
    ] {
        let text = rule_cleanup(asr, Some("en"));
        let request = CleanupRequest {
            text: &text,
            language: Some("en"),
            ..CleanupRequest::default()
        };

        // The first call also loads the model, so it gets a load's worth of extra time.
        let outcome = llm_cleanup(&mut llm, &request, Duration::from_secs(30));

        assert_eq!(outcome.text, expected, "{:?}", outcome.fallback);
    }
}

#[test]
#[ignore = "needs a model"]
fn tokens_stream_and_add_up_to_the_reply() {
    let mut llm = host(KeepLoaded::Always);
    let mut streamed = String::new();
    let prompt = LlmPrompt {
        user: "Count from one to five.".into(),
        max_tokens: 32,
        ..LlmPrompt::default()
    };

    let reply = llm
        .generate(&prompt, &mut |t| streamed.push_str(t))
        .unwrap();

    assert!(!reply.is_empty());
    assert_eq!(streamed, reply);
}

#[test]
#[ignore = "needs a model"]
fn a_real_decode_stops_at_the_deadline_and_cleanup_falls_back() {
    let mut llm = host(KeepLoaded::Always);
    let warm_up = LlmPrompt {
        user: "Hello".into(),
        max_tokens: 1,
        ..LlmPrompt::default()
    };
    llm.generate(&warm_up, &mut |_| {}).unwrap();
    let text = dictation(160);
    let request = CleanupRequest {
        text: &text,
        language: Some("en"),
        ..CleanupRequest::default()
    };

    let started = Instant::now();
    let outcome = llm_cleanup(&mut llm, &request, DEFAULT_TIMEOUT);
    let elapsed = started.elapsed();

    assert!(
        matches!(outcome.fallback, Some(Fallback::Timeout)),
        "{:?}",
        outcome.fallback
    );
    assert_eq!(outcome.text, text);
    assert!(
        elapsed < DEFAULT_TIMEOUT + Duration::from_millis(100),
        "{elapsed:?}"
    );

    // The engine stopped too: the next caller is served, not stuck behind the long decode.
    let started = Instant::now();
    llm.generate(&warm_up, &mut |_| {}).unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
}

#[test]
#[ignore = "needs a model"]
fn idle_unload_gives_the_memory_back_and_the_model_loads_again() {
    let mut llm = host(KeepLoaded::For(Duration::from_millis(200)));
    let prompt = LlmPrompt {
        user: "Hello".into(),
        max_tokens: 8,
        ..LlmPrompt::default()
    };
    let before = resident_kilobytes();

    let started = Instant::now();
    llm.generate(&prompt, &mut |_| {}).unwrap();
    let load = started.elapsed();
    let loaded = resident_kilobytes();
    std::thread::sleep(Duration::from_secs(2));
    let unloaded = resident_kilobytes();

    println!(
        "before {} MB, loaded {} MB, unloaded {} MB, first generation {load:?}",
        before / 1024,
        loaded / 1024,
        unloaded / 1024
    );
    assert!(
        unloaded < before + (loaded - before) / 4,
        "memory not released"
    );

    llm.generate(&prompt, &mut |_| {}).unwrap();
}
