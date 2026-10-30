//! Silero through sherpa-onnx on real audio. The model isn't in the repo, so the tests that
//! need it are ignored by default. Run them with:
//!
//! `WF_SILERO_MODEL=/path/to/silero_vad.onnx cargo test -p wf-pipeline --features silero -- --include-ignored`
#![cfg(feature = "silero")]

use std::path::Path;
use std::thread;
use std::time::Duration;

use wf_core::audio::{AudioClip, FRAME_SAMPLES, SAMPLE_RATE_HZ};
use wf_pipeline::silero::SileroVad;
use wf_pipeline::vad::{LiveVad, VadOptions, speech_chunks, transcribe_speech};
use wf_pipeline::{Asr, AsrOptions, PipelineError, Transcript, Vad};

/// Well above the room-tone fixture, well below speech: a ground truth that isn't Silero.
const AUDIBLE: f32 = 0.02;

fn silero() -> SileroVad {
    let model = std::env::var("WF_SILERO_MODEL").expect("set WF_SILERO_MODEL to silero_vad.onnx");
    SileroVad::new(Path::new(&model)).unwrap()
}

fn fixture(name: &str) -> Vec<f32> {
    let path = format!("{}/tests/fixtures/{name}.wav", env!("CARGO_MANIFEST_DIR"));
    let wave = sherpa_onnx::Wave::read(&path).expect(&path);
    assert_eq!(wave.sample_rate(), SAMPLE_RATE_HZ as i32);
    wave.samples().to_vec()
}

fn silence(ms: usize) -> Vec<f32> {
    vec![0.0; ms * SAMPLE_RATE_HZ as usize / 1_000]
}

/// A `seconds`-long stretch of the room-tone fixture.
fn room_tone(seconds: usize) -> Vec<f32> {
    fixture("room_tone")
        .into_iter()
        .cycle()
        .take(seconds * SAMPLE_RATE_HZ as usize)
        .collect()
}

fn audible(chunks: &[AudioClip]) -> usize {
    chunks
        .iter()
        .flat_map(|chunk| chunk.samples())
        .filter(|sample| sample.abs() > AUDIBLE)
        .count()
}

fn seconds(chunk: &AudioClip) -> f32 {
    chunk.duration().as_secs_f32()
}

#[derive(Default)]
struct CountingAsr {
    calls: usize,
}

impl Asr for CountingAsr {
    fn transcribe(
        &mut self,
        _clip: &AudioClip,
        _options: &AsrOptions,
    ) -> Result<Transcript, PipelineError> {
        self.calls += 1;
        Ok(Transcript {
            text: "hallucination".into(),
            language: None,
        })
    }
}

#[test]
fn a_model_that_is_not_installed_is_reported_as_missing() {
    let missing = Path::new("/nonexistent/silero_vad.onnx");

    let error = SileroVad::new(missing).err().unwrap();

    assert!(matches!(error, PipelineError::ModelMissing(_)), "{error}");
}

fn assert_silence_stays_out_of_asr(recording: Vec<f32>) {
    let recording = AudioClip::new(recording);
    let mut vad = silero();
    let mut asr = CountingAsr::default();

    let chunks = speech_chunks(&recording, Some(&mut vad), &VadOptions::default()).unwrap();
    let transcript = transcribe_speech(chunks, &mut asr, &AsrOptions::default()).unwrap();

    assert_eq!(asr.calls, 0);
    assert_eq!(transcript, None);
}

#[test]
#[ignore = "needs WF_SILERO_MODEL"]
fn digital_silence_never_reaches_asr() {
    assert_silence_stays_out_of_asr(silence(5_000));
}

#[test]
#[ignore = "needs WF_SILERO_MODEL"]
fn room_tone_never_reaches_asr() {
    assert_silence_stays_out_of_asr(room_tone(10));
}

fn surrounded_by_silence(speech: &[f32]) -> AudioClip {
    AudioClip::new([&silence(1_500), speech, &silence(1_500)].concat())
}

fn assert_trims_silence_and_keeps_speech(name: &str) {
    let speech = fixture(name);
    let recording = surrounded_by_silence(&speech);
    let mut vad = silero();

    let chunks = speech_chunks(&recording, Some(&mut vad), &VadOptions::default()).unwrap();

    assert_eq!(chunks.len(), 1);
    assert_eq!(audible(&chunks), audible(std::slice::from_ref(&recording)));
    let spoken = speech.len() as f32 / SAMPLE_RATE_HZ as f32;
    let kept = seconds(&chunks[0]);
    assert!(kept < spoken + 1.0, "kept {kept} s of {spoken} s of speech");

    // The other half of the silence tests: speech does reach ASR.
    let mut asr = CountingAsr::default();
    let transcript = transcribe_speech(chunks, &mut asr, &AsrOptions::default()).unwrap();
    assert_eq!(asr.calls, 1);
    assert!(transcript.is_some());
}

#[test]
#[ignore = "needs WF_SILERO_MODEL"]
fn english_speech_is_trimmed_without_losing_any() {
    assert_trims_silence_and_keeps_speech("speech_en");
}

#[test]
#[ignore = "needs WF_SILERO_MODEL"]
fn italian_speech_is_trimmed_without_losing_any() {
    assert_trims_silence_and_keeps_speech("speech_it");
}

#[test]
#[ignore = "needs WF_SILERO_MODEL"]
fn speech_over_room_tone_is_found() {
    let speech = fixture("speech_en");
    let tone = room_tone(2);
    let noisy: Vec<f32> = speech
        .iter()
        .zip(tone.iter().cycle())
        .map(|(voice, noise)| voice + noise)
        .collect();
    let recording = AudioClip::new([&tone[..], &noisy[..], &tone[..]].concat());
    let mut vad = silero();

    let chunks = speech_chunks(&recording, Some(&mut vad), &VadOptions::default()).unwrap();

    assert_eq!(chunks.len(), 1);
    assert_eq!(audible(&chunks), audible(std::slice::from_ref(&recording)));
}

fn two_sentences_with_a_pause() -> AudioClip {
    let (english, italian) = (fixture("speech_en"), fixture("speech_it"));
    AudioClip::new([&english[..], &silence(1_500), &italian[..]].concat())
}

#[test]
#[ignore = "needs WF_SILERO_MODEL"]
fn a_long_dictation_splits_at_the_real_pause_and_a_short_one_does_not() {
    let recording = two_sentences_with_a_pause();
    let mut vad = silero();
    let split = VadOptions {
        min_chunk: Duration::from_secs(2),
        ..VadOptions::default()
    };

    let chunks = speech_chunks(&recording, Some(&mut vad), &split).unwrap();
    let whole = speech_chunks(&recording, Some(&mut vad), &VadOptions::default()).unwrap();

    assert_eq!(chunks.len(), 2);
    assert_eq!(audible(&chunks), audible(std::slice::from_ref(&recording)));
    assert_eq!(whole.len(), 1);
}

#[test]
#[ignore = "needs WF_SILERO_MODEL"]
fn a_recording_stopped_mid_sentence_keeps_its_last_words() {
    let speech = fixture("speech_en");
    let stopped_early = &speech[..speech.len() * 2 / 3];
    let recording = AudioClip::new([&silence(1_000), stopped_early].concat());
    let mut vad = silero();

    let chunks = speech_chunks(&recording, Some(&mut vad), &VadOptions::default()).unwrap();

    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].samples().last(), recording.samples().last());
    assert_eq!(audible(&chunks), audible(std::slice::from_ref(&recording)));
}

#[test]
#[ignore = "needs WF_SILERO_MODEL"]
fn results_do_not_depend_on_the_recording_before() {
    let recording = surrounded_by_silence(&fixture("speech_en"));
    let mut vad = silero();

    let first = vad.speech_segments(&recording).unwrap();
    // Abandoned mid-speech, as Cancel does.
    vad.accept(&fixture("speech_it")).unwrap();
    vad.finish().unwrap();
    let second = vad.speech_segments(&recording).unwrap();

    assert_eq!(first.len(), 1);
    assert_eq!(first, second);
}

#[test]
#[ignore = "needs WF_SILERO_MODEL"]
fn live_frames_find_the_same_speech_as_the_finished_clip() {
    let recording = surrounded_by_silence(&fixture("speech_en"));
    let mut vad = silero();
    let whole = vad.speech_segments(&recording).unwrap();

    let mut live = Vec::new();
    for frame in recording.samples().chunks(FRAME_SAMPLES) {
        live.extend(vad.accept(frame).unwrap());
    }
    live.extend(vad.finish().unwrap());

    assert_eq!(live, whole);
}

#[test]
#[ignore = "needs WF_SILERO_MODEL"]
fn hands_free_auto_stops_after_the_silence_that_follows_speech() {
    let recording = [fixture("speech_en"), room_tone(5)].concat();
    let spoken = fixture("speech_en").len() as f32 / SAMPLE_RATE_HZ as f32;
    let mut vad = silero();
    let options = VadOptions {
        auto_stop: Some(Duration::from_secs(3)),
        ..VadOptions::default()
    };
    let mut live = LiveVad::new(&mut vad, options);

    let mut stopped_at = None;
    for (index, frame) in recording.chunks(FRAME_SAMPLES).enumerate() {
        if live.push(frame).unwrap().auto_stop {
            stopped_at = Some((index + 1) as f32 * 0.04);
            break;
        }
    }
    let chunks = live.finish().unwrap();

    let stopped_at = stopped_at.expect("auto-stop never fired");
    assert!(
        (spoken + 2.5..spoken + 3.5).contains(&stopped_at),
        "stopped at {stopped_at} s, speech ended by {spoken} s"
    );
    assert_eq!(chunks.len(), 1);
}

#[test]
#[ignore = "needs WF_SILERO_MODEL"]
fn a_silent_hands_free_dictation_auto_stops_with_nothing_to_transcribe() {
    let recording = room_tone(5);
    let mut vad = silero();
    let options = VadOptions {
        auto_stop: Some(Duration::from_secs(3)),
        ..VadOptions::default()
    };
    let mut live = LiveVad::new(&mut vad, options);

    let mut stopped = false;
    for frame in recording.chunks(FRAME_SAMPLES) {
        let update = live.push(frame).unwrap();
        assert!(update.chunks.is_empty());
        stopped |= update.auto_stop;
    }
    let mut asr = CountingAsr::default();
    let chunks = live.finish().unwrap();
    let transcript = transcribe_speech(chunks, &mut asr, &AsrOptions::default()).unwrap();

    assert!(stopped);
    assert_eq!(asr.calls, 0);
    assert_eq!(transcript, None);
}

#[test]
#[ignore = "needs WF_SILERO_MODEL"]
fn runs_boxed_on_a_worker_thread() {
    let mut vad: Box<dyn Vad> = Box::new(silero());
    let recording = surrounded_by_silence(&fixture("speech_en"));

    let segments = thread::spawn(move || vad.speech_segments(&recording).unwrap())
        .join()
        .unwrap();

    assert_eq!(segments.len(), 1);
}
