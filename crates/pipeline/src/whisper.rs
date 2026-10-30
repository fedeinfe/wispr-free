//! whisper.cpp behind [`Asr`] (ADR-0003). Behind the `whisper` feature because it builds
//! whisper.cpp from source. Add `metal`, `cuda` or `vulkan` for the GPU backend.

use std::borrow::Cow;
use std::path::Path;

use wf_core::audio::{AudioClip, SAMPLE_RATE_HZ};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

use crate::{Asr, AsrOptions, PipelineError, Transcript};

/// Above this a segment is noise: 0.93 on digital silence, at most 0.09 on measured speech,
/// quiet speech included. Token confidence doesn't separate them, so it isn't used.
const NO_SPEECH_PROBABILITY: f32 = 0.6;
/// Language detection fails on less audio than this.
const MIN_SAMPLES: usize = SAMPLE_RATE_HZ as usize;

pub struct WhisperAsr {
    state: WhisperState,
    multilingual: bool,
    threads: usize,
}

impl WhisperAsr {
    pub fn new(model: &Path) -> Result<Self, PipelineError> {
        Self::with_threads(model, true, crate::inference_threads())
    }

    /// [`Self::new`] with the GPU and thread count fixed, to measure a smaller machine.
    pub fn with_threads(model: &Path, gpu: bool, threads: usize) -> Result<Self, PipelineError> {
        if !model.is_file() {
            return Err(PipelineError::ModelMissing(model.display().to_string()));
        }
        // Without a log backend these send whisper.cpp's chatter nowhere.
        whisper_rs::install_logging_hooks();
        let mut params = WhisperContextParameters::default();
        params.use_gpu(gpu);
        let context = WhisperContext::new_with_params(model.to_string_lossy().as_ref(), params)
            .map_err(engine)?;
        Ok(Self {
            multilingual: context.is_multilingual(),
            state: context.create_state().map_err(engine)?,
            threads,
        })
    }

    /// The code for whisper.cpp, `None` to auto-detect. English-only models can't detect.
    fn language(&self, options: &AsrOptions) -> Result<Option<String>, PipelineError> {
        if !self.multilingual {
            return Ok(Some("en".to_owned()));
        }
        let Some(code) = &options.language else {
            return Ok(None);
        };
        let primary = code.split(['-', '_']).next().unwrap_or(code).to_lowercase();
        if primary.contains('\0') || whisper_rs::get_lang_id(&primary).is_none() {
            return Err(PipelineError::Engine(format!("unknown language: {code}")));
        }
        Ok(Some(primary))
    }
}

/// Whisper reads the prompt as text said just before, so a word list steers its spelling.
fn initial_prompt(bias_words: &[String]) -> String {
    let words: Vec<String> = bias_words
        .iter()
        .map(|word| word.replace('\0', ""))
        .filter(|word| !word.trim().is_empty())
        .collect();
    words.join(", ")
}

impl Asr for WhisperAsr {
    fn transcribe(
        &mut self,
        clip: &AudioClip,
        options: &AsrOptions,
    ) -> Result<Transcript, PipelineError> {
        if clip.samples().is_empty() {
            return Ok(Transcript {
                language: options.language.clone(),
                ..Transcript::default()
            });
        }
        let language = self.language(options)?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(language.as_deref());
        params.set_n_threads(self.threads as i32);
        params.set_no_context(true);
        // Otherwise silence comes back as the text "[BLANK_AUDIO]".
        params.set_suppress_nst(true);
        let prompt = initial_prompt(&options.bias_words);
        if !prompt.is_empty() {
            params.set_initial_prompt(&prompt);
        }
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_special(false);
        params.set_print_timestamps(false);
        let mut audio = Cow::Borrowed(clip.samples());
        if audio.len() < MIN_SAMPLES {
            audio.to_mut().resize(MIN_SAMPLES, 0.0);
        }
        self.state.full(params, &audio).map_err(engine)?;

        let mut text = String::new();
        for segment in self.state.as_iter() {
            if segment.no_speech_probability() < NO_SPEECH_PROBABILITY {
                text.push_str(&segment.to_str_lossy().map_err(engine)?);
            }
        }
        Ok(Transcript {
            text: text.trim().to_owned(),
            language: whisper_rs::get_lang_str(self.state.full_lang_id_from_state())
                .map(str::to_owned),
        })
    }
}

fn engine(error: impl std::fmt::Display) -> PipelineError {
    PipelineError::Engine(error.to_string())
}
