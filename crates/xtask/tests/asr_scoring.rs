//! The ASR benchmark's arithmetic and script, without a model.

use xtask::asr_score::{BenchReport, ClipResult, Mode, median, render, summarize};
use xtask::meeting_eval::word_edits;
use xtask::speech_fixture::{Length, Source, SpeechScript, with_noise};

fn result(
    language: &str,
    source: Source,
    length: Length,
    edits: usize,
    words: usize,
) -> ClipResult {
    ClipResult {
        id: format!("{language}-clip"),
        language: language.to_owned(),
        source,
        length,
        seconds: 10.0,
        mode: Mode::Auto,
        times: vec![0.5, 0.1, 0.3],
        detected: Some(language.to_owned()),
        text: String::new(),
        varied: false,
        edits,
        words,
    }
}

#[test]
fn the_middle_value_ignores_a_single_outlier() {
    assert_eq!(median(&[0.1, 9.0, 0.2]), 0.2);
    assert_eq!(median(&[1.0, 3.0]), 2.0);
    assert_eq!(median(&[]), 0.0);
}

#[test]
fn wer_is_pooled_so_a_long_clip_weighs_more_than_a_short_one() {
    let results = [
        result("en", Source::Clean, Length::Short, 1, 10),
        result("en", Source::Clean, Length::Long, 3, 90),
    ];

    let rows = summarize(&results);

    assert_eq!(rows.len(), 1);
    assert!((rows[0].wer - 0.04).abs() < 1e-9, "{rows:?}");
    assert_eq!(rows[0].short_seconds, Some(0.3));
    assert!((rows[0].long_rtf.unwrap() - 0.03).abs() < 1e-9, "{rows:?}");
}

#[test]
fn languages_and_kinds_of_audio_stay_apart() {
    let results = [
        result("en", Source::Clean, Length::Short, 0, 10),
        result("en", Source::Noisy, Length::Short, 5, 10),
        result("it", Source::Clean, Length::Short, 1, 10),
    ];

    let rows = summarize(&results);

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1].wer, 0.5);
    assert_eq!(rows[2].language, "it");
}

#[test]
fn a_clip_heard_in_the_wrong_language_is_counted() {
    let mut wrong = result("it", Source::Clean, Length::Short, 10, 10);
    wrong.detected = Some("es".to_owned());
    let mut unknown = result("it", Source::Clean, Length::Short, 0, 10);
    unknown.detected = None;

    let rows = summarize(&[wrong, unknown]);

    assert_eq!(rows[0].wrong_language, 1);
}

#[test]
fn the_report_says_when_a_rerun_changed_the_text() {
    let mut clip = result("en", Source::Clean, Length::Short, 0, 10);
    clip.varied = true;

    let text = render(&BenchReport {
        model: "m".to_owned(),
        cpu_threads: Some(4),
        load_seconds: 0.5,
        clips: vec![clip],
    });

    assert!(text.contains("on 4 CPU threads"), "{text}");
    assert!(text.contains("changed between runs: 1"), "{text}");
}

#[test]
fn edits_ignore_case_and_punctuation_and_count_the_reference() {
    assert_eq!(word_edits("Ciao, Giulia!", "ciao giulia"), (0, 2));
    assert_eq!(word_edits("the cat sat", "the dog"), (2, 3));
}

#[test]
fn noise_lands_at_the_asked_level_and_repeats() {
    let speech: Vec<f32> = (0..16_000).map(|n| (n as f32 * 0.05).sin() * 0.3).collect();

    let noisy = with_noise(&speech, 12.0);

    let power = |samples: &[f32]| samples.iter().map(|s| s * s).sum::<f32>();
    let added: Vec<f32> = noisy.iter().zip(&speech).map(|(a, b)| a - b).collect();
    let snr = 10.0 * (power(&speech) / power(&added)).log10();
    assert!((snr - 12.0).abs() < 0.1, "{snr}");
    assert_eq!(noisy, with_noise(&speech, 12.0));
}

#[test]
fn the_script_covers_both_languages_at_both_lengths_without_digits() {
    let script = SpeechScript::bundled().unwrap();
    for language in ["en", "it"] {
        let passages = || script.passage.iter().filter(|p| p.language == language);
        assert!(!script.voices[language].is_empty());
        assert!(passages().filter(|p| p.length == Length::Short).count() >= 10);
        assert!(passages().filter(|p| p.length == Length::Long).count() >= 2);
        assert!(passages().all(|p| !p.text.contains(|c: char| c.is_ascii_digit())));
    }
    assert!(script.recording.iter().all(|r| !r.text.is_empty()));
}
