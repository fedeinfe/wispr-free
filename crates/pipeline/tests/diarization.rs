//! sherpa-onnx diarization on real voices: the committed English and Italian recordings are two
//! different readers. The models aren't in the repo, so these are skipped by default:
//!
//! `WF_SEGMENTATION_MODEL=model.int8.onnx WF_EMBEDDING_MODEL=campplus.onnx cargo test -p wf-pipeline --release --features diarization --test diarization -- --ignored`
#![cfg(feature = "diarization")]

mod support;

use std::path::Path;
use std::time::Duration;

use support::fixture;
use wf_core::audio::{AudioClip, SAMPLE_RATE_HZ};
use wf_pipeline::diarization::{DiarizationConfig, SherpaDiarizer};
use wf_pipeline::{Diarizer, PipelineError, SpeakerTurn};

fn path(variable: &str) -> String {
    std::env::var(variable).unwrap_or_else(|_| panic!("set {variable}"))
}

fn diarizer() -> SherpaDiarizer {
    let config = DiarizationConfig::new(path("WF_SEGMENTATION_MODEL"), path("WF_EMBEDDING_MODEL"));
    SherpaDiarizer::new(&config).unwrap()
}

fn silence(seconds: usize) -> Vec<f32> {
    vec![0.0; seconds * SAMPLE_RATE_HZ as usize]
}

/// Alternating readers: English, Italian, English, Italian. Returns the clip and where each
/// reader's stretches are.
fn two_readers() -> (AudioClip, Vec<(&'static str, Duration, Duration)>) {
    let english = fixture("speech_mid_phrase_en");
    let italian = fixture("speech_it");
    let mut samples = Vec::new();
    let mut truth = Vec::new();
    for (name, clip) in [
        ("en", &english),
        ("it", &italian),
        ("en", &english),
        ("it", &italian),
    ] {
        let start = Duration::from_secs_f64(samples.len() as f64 / f64::from(SAMPLE_RATE_HZ));
        samples.extend_from_slice(clip.samples());
        truth.push((name, start, start + clip.duration()));
        samples.extend(silence(2));
    }
    (AudioClip::new(samples), truth)
}

/// The speaker heard for longest within a stretch.
fn dominant(turns: &[SpeakerTurn], start: Duration, end: Duration) -> Option<u32> {
    let mut totals = std::collections::BTreeMap::new();
    for turn in turns {
        let overlap = turn.end.min(end).saturating_sub(turn.start.max(start));
        if !overlap.is_zero() {
            *totals.entry(turn.speaker).or_insert(Duration::ZERO) += overlap;
        }
    }
    totals
        .into_iter()
        .max_by_key(|&(_, total)| total)
        .map(|(speaker, _)| speaker)
}

#[test]
#[ignore = "needs models"]
fn two_readers_get_two_speakers_and_keep_them_when_they_return() {
    let (clip, truth) = two_readers();

    let turns = diarizer().diarize(&clip).unwrap();

    let heard: Vec<u32> = truth
        .iter()
        .map(|&(_, start, end)| dominant(&turns, start, end).expect("speech was heard"))
        .collect();
    assert_eq!(heard[0], heard[2], "{turns:?}");
    assert_eq!(heard[1], heard[3], "{turns:?}");
    assert_ne!(heard[0], heard[1], "{turns:?}");
}

#[test]
#[ignore = "needs models"]
fn silence_has_no_turns() {
    let clip = AudioClip::new(silence(30));

    assert!(diarizer().diarize(&clip).unwrap().is_empty());
}

#[test]
#[ignore = "needs models"]
fn an_empty_clip_has_no_turns() {
    assert!(
        diarizer()
            .diarize(&AudioClip::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
#[ignore = "needs models"]
fn the_same_audio_gives_the_same_turns() {
    let (clip, _) = two_readers();
    let mut diarizer = diarizer();

    assert_eq!(
        diarizer.diarize(&clip).unwrap(),
        diarizer.diarize(&clip).unwrap()
    );
}

#[test]
fn a_missing_model_is_reported_as_missing() {
    let config = DiarizationConfig::new(
        Path::new("/nonexistent/segmentation.onnx"),
        "/nonexistent/embedding.onnx",
    );

    assert!(matches!(
        SherpaDiarizer::new(&config),
        Err(PipelineError::ModelMissing(_))
    ));
}
