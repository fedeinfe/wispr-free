//! Holding a recording to one language. Whisper works out the language from the audio it's given,
//! and on a second or two it guesses: one Italian meeting came back in Russian, Polish, French and
//! Spanish, chunk by chunk. So a longer stretch of speech decides for the chunks too short to say.
//! A chunk long enough to judge for itself can still change it, which is how a bilingual meeting
//! keeps up.

use std::time::Duration;

use wf_core::audio::AudioClip;

use crate::{Asr, AsrOptions, PipelineError, Transcript};

/// Enough speech to trust the detection. Thirteen voices in nine languages were all right from
/// 5 s and some were wrong at 3 s, and a call is harder than those recordings.
pub const SETTLING_SPEECH: Duration = Duration::from_secs(8);

/// The language so far. `None` until something decides it, and then short chunks use it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SteadyLanguage {
    language: Option<String>,
}

impl SteadyLanguage {
    /// Decides the language from `speech`. Does nothing if it's decided, or if `options` fix it.
    /// An engine that doesn't report a language leaves it undecided.
    pub fn settle(
        &mut self,
        asr: &mut dyn Asr,
        speech: &AudioClip,
        options: &AsrOptions,
    ) -> Result<(), PipelineError> {
        if self.language.is_none() && options.language.is_none() {
            self.language = asr.transcribe(speech, options)?.language;
        }
        Ok(())
    }

    /// Transcribes `clip`, in the language so far unless it's long enough to go by itself.
    pub fn transcribe(
        &mut self,
        asr: &mut dyn Asr,
        clip: &AudioClip,
        options: &AsrOptions,
    ) -> Result<Transcript, PipelineError> {
        let short = clip.duration() < SETTLING_SPEECH;
        let transcript = if short && options.language.is_none() {
            let held = AsrOptions {
                language: self.language.clone(),
                ..options.clone()
            };
            asr.transcribe(clip, &held)?
        } else {
            asr.transcribe(clip, options)?
        };
        if !short && transcript.language.is_some() {
            self.language.clone_from(&transcript.language);
        }
        Ok(transcript)
    }
}
