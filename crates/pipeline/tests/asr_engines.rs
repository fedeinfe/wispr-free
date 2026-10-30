//! What every ASR engine must do, run against the real engines on the committed speech
//! fixtures. The models aren't in the repo, so these tests are ignored by default. Run them with:
//!
//! `WF_WHISPER_MODEL=ggml-small-q8_0.bin WF_WHISPER_EN_MODEL=ggml-base.en-q5_1.bin WF_PARAKEET_DIR=parakeet-tdt-0.6b-v3-int8 WF_SILERO_MODEL=silero_vad.onnx cargo test -p wf-pipeline --release --features whisper,sherpa,silero --test asr_engines -- --include-ignored`
#![cfg(any(feature = "whisper", feature = "sherpa"))]

mod support;

use std::thread;

use support::fixture;
use wf_core::audio::{AudioClip, SAMPLE_RATE_HZ};
use wf_pipeline::{Asr, AsrOptions};

/// Lowercase words only, so engines can differ on punctuation and casing.
fn words(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn heard(asr: &mut dyn Asr, clip: &AudioClip, options: &AsrOptions) -> String {
    words(&asr.transcribe(clip, options).unwrap().text)
}

fn biased(bias_words: &[&str]) -> AsrOptions {
    AsrOptions {
        bias_words: bias_words.iter().map(|word| (*word).to_owned()).collect(),
        ..AsrOptions::default()
    }
}

fn hears_english(asr: &mut dyn Asr) {
    let text = heard(asr, &fixture("speech_en"), &AsrOptions::default());
    assert!(text.contains("selected and translated by"), "{text}");
}

fn hears_italian(asr: &mut dyn Asr) {
    let text = heard(asr, &fixture("speech_it"), &AsrOptions::default());
    assert!(text.contains("registrazioni"), "{text}");
    assert!(text.contains("dominio pubblico"), "{text}");
}

/// Real speech must never vanish without an error: Parakeet used to return nothing here.
fn keeps_the_words_of_a_clip_cut_mid_phrase(asr: &mut dyn Asr) {
    let text = heard(
        asr,
        &fixture("speech_mid_phrase_en"),
        &AsrOptions::default(),
    );

    for word in ["wise", "eyes", "notice", "exercise", "elephant", "government"] {
        assert!(text.contains(word), "no {word:?} in {text:?}");
    }
}

fn empty_audio_has_no_words(asr: &mut dyn Asr) {
    let transcript = asr
        .transcribe(&AudioClip::default(), &AsrOptions::default())
        .unwrap();
    assert_eq!(transcript.text, "");
}

/// The speaker says "Coxwell"; a dictionary can pull the spelling to a sound-alike.
fn a_bias_word_changes_what_is_heard(asr: &mut dyn Asr, sound_alike: &str, plain: &str) {
    let clip = fixture("speech_en");
    assert!(heard(asr, &clip, &AsrOptions::default()).contains(plain));

    let text = heard(asr, &clip, &biased(&[sound_alike]));

    assert!(text.contains(&sound_alike.to_lowercase()), "{text}");
}

/// Context biasing passes many on-screen words the speaker never says.
fn unrelated_bias_words_add_nothing(asr: &mut dyn Asr) {
    let clip = fixture("speech_it");
    let plain = heard(asr, &clip, &AsrOptions::default());
    let unrelated = [
        "Kubernetes",
        "Postgres",
        "invoice",
        "quarterly",
        "Slack",
        "roadmap",
        "Grafana",
        "deployment",
        "sprint",
        "backlog",
        "Berlin",
        "tomorrow",
        "budget",
        "Figma",
        "Jira",
    ];

    let text = heard(asr, &clip, &biased(&unrelated));

    assert!(text.contains("registrazioni"), "{text}");
    for word in unrelated {
        let word = word.to_lowercase();
        assert!(!text.contains(&word) || plain.contains(&word), "{text}");
    }
}

/// Bias words come from the user's dictionary and from whatever is on screen.
fn any_bias_text_is_safe(asr: &mut dyn Asr) {
    let clip = fixture("speech_en");
    let awkward = [
        "",
        " ",
        "a\0b",
        "/",
        "a/b",
        ":2.0",
        "#1",
        "@me",
        "日本語",
        "🎙️",
        "C++",
        "\n",
        "x\ny",
    ];

    let text = heard(asr, &clip, &biased(&awkward));

    assert!(text.contains("selected and translated"), "{text}");
}

fn a_very_long_bias_list_is_safe(asr: &mut dyn Asr) {
    let many: Vec<String> = (0..2_000).map(|n| format!("word{n}")).collect();
    let options = AsrOptions {
        bias_words: many,
        ..AsrOptions::default()
    };

    let text = heard(asr, &fixture("speech_en"), &options);

    assert!(text.contains("selected and translated"), "{text}");
}

/// A VAD chunk can be tiny, and with VAD off so can a whole dictation.
fn a_very_short_clip_is_safe(asr: &mut dyn Asr) {
    for samples in [1, 80, 160, 400, 1_600, 4_000] {
        let clip =
            AudioClip::new(fixture("speech_en").samples()[16_000..16_000 + samples].to_vec());
        asr.transcribe(&clip, &AsrOptions::default()).unwrap();
    }
}

/// VAD is optional, so silence can reach ASR, and any invented text would get typed.
fn silence_has_no_words(asr: &mut dyn Asr) {
    let silence = AudioClip::new(vec![0.0; 5 * SAMPLE_RATE_HZ as usize]);

    assert_eq!(heard(asr, &silence, &AsrOptions::default()), "");
}

fn each_clip_is_independent_of_the_last(asr: &mut dyn Asr) {
    let english = fixture("speech_en");
    let italian = fixture("speech_it");
    let options = AsrOptions::default();

    let first = heard(asr, &english, &options);
    heard(asr, &italian, &options);
    let again = heard(asr, &english, &options);

    assert_eq!(first, again);
}

fn works_boxed_on_a_worker_thread(mut asr: Box<dyn Asr>) {
    let text =
        thread::spawn(move || heard(asr.as_mut(), &fixture("speech_en"), &AsrOptions::default()))
            .join()
            .unwrap();

    assert!(text.contains("selected and translated"), "{text}");
}

/// A real recording through the whole live path: Silero closes chunks, the worker transcribes.
#[cfg(feature = "silero")]
fn streams_a_recording_with_timestamps(asr: Box<dyn Asr>) {
    use std::time::Duration;

    use wf_core::audio::FRAME_SAMPLES;
    use wf_pipeline::silero::SileroVad;
    use wf_pipeline::vad::{LiveVad, VadOptions};
    use wf_pipeline::worker::AsrWorker;

    let model = std::env::var("WF_SILERO_MODEL").expect("set WF_SILERO_MODEL");
    let mut vad = SileroVad::new(std::path::Path::new(&model)).unwrap();
    let (english, italian) = (fixture("speech_en"), fixture("speech_it"));
    let pause = vec![0.0; 3 * SAMPLE_RATE_HZ as usize];
    let italian_starts = english.duration() + Duration::from_secs(3);
    let recording = [english.samples(), &pause, italian.samples()].concat();

    let worker = AsrWorker::spawn(asr, AsrOptions::default());
    let options = VadOptions {
        min_chunk: Duration::from_secs(1),
        ..VadOptions::default()
    };
    let mut live = LiveVad::new(&mut vad, options);
    for frame in recording.chunks(FRAME_SAMPLES) {
        for chunk in live.push(frame).unwrap().chunks {
            worker.submit((), chunk).unwrap();
        }
    }
    for chunk in live.finish().unwrap() {
        worker.submit((), chunk).unwrap();
    }
    let (results, _asr) = worker.finish().unwrap();

    let results: Vec<_> = results.into_iter().map(Result::unwrap).collect();
    assert_eq!(results.len(), 2, "{results:?}");
    assert!(words(&results[0].transcript.text).contains("selected and translated"));
    assert!(words(&results[1].transcript.text).contains("dominio pubblico"));
    assert!(results[0].start < Duration::from_secs(2), "{results:?}");
    let drift = results[1].start.abs_diff(italian_starts);
    assert!(drift < Duration::from_secs(1), "{results:?}");
    assert!(results[1].end <= italian_starts + italian.duration() + Duration::from_millis(50));
}

macro_rules! engine_contract {
    ($module:ident, $feature:literal, $engine:expr, $sound_alike:literal, $plain:literal) => {
        #[cfg(feature = $feature)]
        mod $module {
            use super::*;

            #[test]
            #[ignore = "needs a model"]
            fn hears_english() {
                super::hears_english(&mut $engine);
            }

            #[test]
            #[ignore = "needs a model"]
            fn hears_italian() {
                super::hears_italian(&mut $engine);
            }

            #[test]
            #[ignore = "needs a model"]
            fn keeps_the_words_of_a_clip_cut_mid_phrase() {
                super::keeps_the_words_of_a_clip_cut_mid_phrase(&mut $engine);
            }

            #[test]
            #[ignore = "needs a model"]
            fn empty_audio_has_no_words() {
                super::empty_audio_has_no_words(&mut $engine);
            }

            #[test]
            #[ignore = "needs a model"]
            fn a_bias_word_changes_what_is_heard() {
                super::a_bias_word_changes_what_is_heard(&mut $engine, $sound_alike, $plain);
            }

            #[test]
            #[ignore = "needs a model"]
            fn unrelated_bias_words_add_nothing() {
                super::unrelated_bias_words_add_nothing(&mut $engine);
            }

            #[test]
            #[ignore = "needs a model"]
            fn any_bias_text_is_safe() {
                super::any_bias_text_is_safe(&mut $engine);
            }

            #[test]
            #[ignore = "needs a model"]
            fn a_very_long_bias_list_is_safe() {
                super::a_very_long_bias_list_is_safe(&mut $engine);
            }

            #[test]
            #[ignore = "needs a model"]
            fn a_very_short_clip_is_safe() {
                super::a_very_short_clip_is_safe(&mut $engine);
            }

            #[test]
            #[ignore = "needs a model"]
            fn silence_has_no_words() {
                super::silence_has_no_words(&mut $engine);
            }

            #[test]
            #[ignore = "needs a model"]
            fn each_clip_is_independent_of_the_last() {
                super::each_clip_is_independent_of_the_last(&mut $engine);
            }

            #[cfg(feature = "silero")]
            #[test]
            #[ignore = "needs a model"]
            fn streams_a_recording_with_timestamps() {
                super::streams_a_recording_with_timestamps(Box::new($engine));
            }

            #[test]
            #[ignore = "needs a model"]
            fn works_boxed_on_a_worker_thread() {
                super::works_boxed_on_a_worker_thread(Box::new($engine));
            }
        }
    };
}

#[cfg(feature = "whisper")]
fn whisper() -> wf_pipeline::whisper::WhisperAsr {
    let model = std::env::var("WF_WHISPER_MODEL").expect("set WF_WHISPER_MODEL to a ggml model");
    wf_pipeline::whisper::WhisperAsr::new(std::path::Path::new(&model)).unwrap()
}

#[cfg(feature = "sherpa")]
fn parakeet() -> wf_pipeline::sherpa::SherpaAsr {
    let dir = std::env::var("WF_PARAKEET_DIR").expect("set WF_PARAKEET_DIR to the model folder");
    wf_pipeline::sherpa::SherpaAsr::new(std::path::Path::new(&dir)).unwrap()
}

engine_contract!(
    whisper_contract,
    "whisper",
    whisper(),
    "Coxwell",
    "cockswell"
);
engine_contract!(
    parakeet_contract,
    "sherpa",
    parakeet(),
    "Cockswell",
    "coxwell"
);

#[cfg(feature = "sherpa")]
mod parakeet_only {
    use wf_pipeline::PipelineError;
    use wf_pipeline::sherpa::SherpaAsr;

    use super::*;

    #[test]
    #[ignore = "needs a model"]
    fn room_tone_has_no_words() {
        let text = heard(
            &mut parakeet(),
            &fixture("room_tone"),
            &AsrOptions::default(),
        );

        assert_eq!(text, "");
    }

    /// One word Parakeet's vocabulary can't spell must not cost the rest of the dictionary.
    #[test]
    #[ignore = "needs a model"]
    fn a_word_outside_the_vocabulary_does_not_lose_the_others() {
        let options = biased(&["日本語", "Cockswell", "🎙️"]);

        let text = heard(&mut parakeet(), &fixture("speech_en"), &options);

        assert!(text.contains("cockswell"), "{text}");
    }

    #[test]
    #[ignore = "needs a model"]
    fn the_language_it_was_told_is_the_language_it_reports() {
        let options = AsrOptions {
            language: Some("it".to_owned()),
            ..AsrOptions::default()
        };

        let transcript = parakeet()
            .transcribe(&fixture("speech_it"), &options)
            .unwrap();

        assert_eq!(transcript.language.as_deref(), Some("it"));
    }

    #[test]
    fn a_missing_model_names_the_file() {
        let folder = tempfile::tempdir().unwrap();

        let error = SherpaAsr::new(folder.path()).err().unwrap();

        assert!(matches!(error, PipelineError::ModelMissing(_)), "{error}");
        assert!(error.to_string().contains("onnx"), "{error}");
    }

    /// sherpa-onnx loads with ONNX Runtime, which throws on a bad file.
    #[test]
    fn a_corrupt_model_is_an_error_not_a_crash() {
        let folder = tempfile::tempdir().unwrap();
        for name in ["encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx"] {
            std::fs::write(folder.path().join(name), b"not a model").unwrap();
        }
        std::fs::write(folder.path().join("tokens.txt"), "<unk> 0\n▁a 1\n").unwrap();

        let error = SherpaAsr::new(folder.path()).err().unwrap();

        assert!(matches!(error, PipelineError::Engine(_)), "{error}");
    }
}

#[cfg(feature = "whisper")]
mod whisper_only {
    use std::path::Path;

    use wf_pipeline::PipelineError;
    use wf_pipeline::whisper::WhisperAsr;

    use super::*;

    fn in_language(language: &str) -> AsrOptions {
        AsrOptions {
            language: Some(language.to_owned()),
            ..AsrOptions::default()
        }
    }

    #[test]
    #[ignore = "needs a model"]
    fn auto_detection_reports_the_language_it_heard() {
        let mut asr = whisper();
        let options = AsrOptions::default();

        let english = asr.transcribe(&fixture("speech_en"), &options).unwrap();
        let italian = asr.transcribe(&fixture("speech_it"), &options).unwrap();

        assert_eq!(english.language.as_deref(), Some("en"));
        assert_eq!(italian.language.as_deref(), Some("it"));
    }

    #[test]
    #[ignore = "needs a model"]
    fn a_fixed_language_overrides_detection() {
        let transcript = whisper()
            .transcribe(&fixture("speech_it"), &in_language("en"))
            .unwrap();

        assert_eq!(transcript.language.as_deref(), Some("en"));
        assert!(!words(&transcript.text).contains("dominio pubblico"));
    }

    #[test]
    #[ignore = "needs a model"]
    fn a_regional_language_code_means_its_language() {
        let transcript = whisper()
            .transcribe(&fixture("speech_it"), &in_language("it-IT"))
            .unwrap();

        assert_eq!(transcript.language.as_deref(), Some("it"));
        assert!(words(&transcript.text).contains("dominio pubblico"));
    }

    #[test]
    #[ignore = "needs a model"]
    fn an_unknown_language_is_an_error_not_a_guess() {
        for language in ["xx", "not a language", "a\0b"] {
            let error = whisper()
                .transcribe(&fixture("speech_it"), &in_language(language))
                .unwrap_err();

            assert!(matches!(error, PipelineError::Engine(_)), "{error}");
        }
    }

    /// The catalog's Lite option. These models can't detect a language, so asking one to fails.
    #[test]
    #[ignore = "needs a model"]
    fn an_english_only_model_needs_no_language() {
        let model = std::env::var("WF_WHISPER_EN_MODEL").expect("set WF_WHISPER_EN_MODEL");
        let mut asr = WhisperAsr::new(Path::new(&model)).unwrap();

        let transcript = asr
            .transcribe(&fixture("speech_en"), &AsrOptions::default())
            .unwrap();

        assert_eq!(transcript.language.as_deref(), Some("en"));
        assert!(words(&transcript.text).contains("selected and translated"));
    }

    #[test]
    fn a_missing_model_names_the_file() {
        let error = WhisperAsr::new(Path::new("/no/such/ggml.bin"))
            .err()
            .unwrap();

        assert_eq!(error.to_string(), "model not installed: /no/such/ggml.bin");
    }

    #[test]
    fn a_corrupt_model_is_an_error_not_a_crash() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"not a whisper model").unwrap();

        let error = WhisperAsr::new(file.path()).err().unwrap();

        assert!(matches!(error, PipelineError::Engine(_)), "{error}");
    }
}
