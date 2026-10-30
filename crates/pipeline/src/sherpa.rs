//! Parakeet-class NeMo transducers through sherpa-onnx (ADR-0003). Behind the `sherpa` feature
//! because sherpa-onnx fetches prebuilt native libraries at build time.

use std::borrow::Cow;
use std::fs;
use std::io::Write;
use std::path::Path;

use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig};
use wf_core::audio::{AudioClip, SAMPLE_RATE_HZ};

use crate::{Asr, AsrOptions, PipelineError, Transcript};

const ENCODER: &str = "encoder.int8.onnx";
const DECODER: &str = "decoder.int8.onnx";
const JOINER: &str = "joiner.int8.onnx";
const TOKENS: &str = "tokens.txt";
/// Hotwords need beam search. It costs about 4 % over greedy on the fixtures.
const DECODING_METHOD: &str = "modified_beam_search";
/// Measured on a sound-alike: 1.5 changes nothing, 3 picks the word, 5 invents repeats of it.
const HOTWORD_SCORE: f32 = 3.0;
/// Without this the decoder can stay on blank and drop the start of a clip, or all of it, with no
/// error: 19 of 70 real clips came back empty, none at 3. At 4.5 it invents speech in noise.
const BLANK_PENALTY: f32 = 3.0;
/// Less audio than this makes the encoder throw, and the exception aborts the process.
const MIN_SAMPLES: usize = SAMPLE_RATE_HZ as usize;

pub struct SherpaAsr {
    recognizer: OfflineRecognizer,
}

impl SherpaAsr {
    /// `dir` holds the model's encoder, decoder, joiner and tokens, as the catalog installs them.
    pub fn new(dir: &Path) -> Result<Self, PipelineError> {
        Self::with_threads(dir, crate::inference_threads())
    }

    /// [`Self::new`] with the thread count fixed, to measure a smaller machine.
    pub fn with_threads(dir: &Path, threads: usize) -> Result<Self, PipelineError> {
        let file = |name: &str| {
            let path = dir.join(name);
            if path.is_file() {
                Ok(path.to_string_lossy().into_owned())
            } else {
                Err(PipelineError::ModelMissing(path.display().to_string()))
            }
        };
        let (encoder, decoder, joiner) = (file(ENCODER)?, file(DECODER)?, file(JOINER)?);
        let tokens_path = file(TOKENS)?;
        let tokens = fs::read_to_string(&tokens_path).map_err(engine)?;

        // sherpa reads this once, while it loads.
        let mut vocab = tempfile::NamedTempFile::new().map_err(engine)?;
        vocab
            .write_all(bpe_vocab(&tokens).as_bytes())
            .map_err(engine)?;

        let mut config = OfflineRecognizerConfig::default();
        config.model_config.transducer = OfflineTransducerModelConfig {
            encoder: Some(encoder),
            decoder: Some(decoder),
            joiner: Some(joiner),
        };
        config.model_config.tokens = Some(tokens_path);
        config.model_config.model_type = Some("nemo_transducer".into());
        config.model_config.modeling_unit = Some("bpe".into());
        config.model_config.bpe_vocab = Some(vocab.path().to_string_lossy().into_owned());
        config.model_config.num_threads = threads as i32;
        config.decoding_method = Some(DECODING_METHOD.into());
        config.hotwords_score = HOTWORD_SCORE;
        config.blank_penalty = BLANK_PENALTY;

        let recognizer = OfflineRecognizer::create(&config)
            .ok_or_else(|| PipelineError::Engine("the sherpa-onnx model failed to load".into()))?;
        Ok(Self { recognizer })
    }
}

impl Asr for SherpaAsr {
    fn transcribe(
        &mut self,
        clip: &AudioClip,
        options: &AsrOptions,
    ) -> Result<Transcript, PipelineError> {
        let mut text = String::new();
        if !clip.samples().is_empty() {
            let hotwords = hotwords(&options.bias_words);
            let stream = if hotwords.is_empty() {
                self.recognizer.create_stream()
            } else {
                self.recognizer.create_stream_with_hotwords(&hotwords)
            };
            let mut audio = Cow::Borrowed(clip.samples());
            if audio.len() < MIN_SAMPLES {
                audio.to_mut().resize(MIN_SAMPLES, 0.0);
            }
            stream.accept_waveform(SAMPLE_RATE_HZ as i32, &audio);
            self.recognizer.decode(&stream);
            let result = stream
                .get_result()
                .ok_or_else(|| PipelineError::Engine("sherpa-onnx returned no result".into()))?;
            text = result.text.trim().to_owned();
        }
        // Parakeet follows the speaker's language by itself and doesn't report it.
        Ok(Transcript {
            text,
            language: options.language.clone(),
        })
    }
}

/// sherpa's per-stream format. A NUL would panic in the binding.
fn hotwords(bias_words: &[String]) -> String {
    let words: Vec<String> = bias_words
        .iter()
        .map(|word| word.replace('\0', "").trim().to_owned())
        .filter(|word| !word.is_empty())
        .collect();
    words.join("/")
}

/// A sentencepiece vocabulary for sherpa's hotword encoder, which the model doesn't ship. Equal
/// scores make it spell a word in the fewest pieces.
fn bpe_vocab(tokens: &str) -> String {
    tokens
        .lines()
        .filter_map(|line| line.rsplit_once(' '))
        .map(|(piece, _id)| piece)
        .filter(|piece| !piece.starts_with('<'))
        .map(|piece| format!("{piece}\t-1\n"))
        .collect()
}

fn engine(error: impl std::fmt::Display) -> PipelineError {
    PipelineError::Engine(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::bpe_vocab;

    #[test]
    fn the_vocabulary_keeps_word_pieces_and_drops_special_tokens() {
        let tokens = "<unk> 0\n<|nospeech|> 1\n▁Co 2871\nx 7950\n▁ 7863\n";

        assert_eq!(bpe_vocab(tokens), "▁Co\t-1\nx\t-1\n▁\t-1\n");
    }
}
