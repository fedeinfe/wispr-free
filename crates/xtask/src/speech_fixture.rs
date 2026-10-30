//! Builds the ASR benchmark's audio from `eval/speech.toml` with macOS `say`: each passage in each
//! voice of its language, clean and with noise, plus `clips.json` with the text of every clip.
//! Spoken passages are cached by content, so a second run takes seconds.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use wf_core::audio::SAMPLE_RATE_HZ;

use crate::meeting_fixture::{read_wav, say, spoken_path, write_wav};

const SCRIPT: &str = include_str!("../eval/speech.toml");
/// The voice's own rate. Adjusted per clip, so a short passage lasts the spec's 10 s.
const SPEAKING_RATE: u32 = 175;
const TARGET_SECONDS: f32 = 10.0;
/// Speech against noise: a loud room or a laptop's own mic, enough to separate the models.
const NOISE_SNR_DB: f32 = 12.0;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeechScript {
    pub voices: BTreeMap<String, Vec<String>>,
    pub recording: Vec<Recording>,
    pub passage: Vec<Passage>,
}

/// A real person's voice, from the pipeline's test fixtures.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recording {
    pub id: String,
    pub language: String,
    pub text: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Passage {
    pub id: String,
    pub language: String,
    pub length: Length,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Length {
    Short,
    Long,
}

impl SpeechScript {
    pub fn bundled() -> anyhow::Result<Self> {
        toml::from_str(SCRIPT).context("parsing eval/speech.toml")
    }
}

/// How the clip was made, which is how results are grouped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// A real person.
    Recording,
    Clean,
    Noisy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clip {
    pub id: String,
    pub language: String,
    pub source: Source,
    pub length: Length,
    pub seconds: f32,
    /// Relative to the fixture's folder, or absolute for a recording.
    pub path: PathBuf,
    pub text: String,
}

/// Reads `clips.json` from a built fixture.
pub fn clips(fixture: &Path) -> anyhow::Result<Vec<Clip>> {
    let path = fixture.join("clips.json");
    let text = fs::read_to_string(&path).with_context(|| path.display().to_string())?;
    Ok(serde_json::from_str(&text)?)
}

/// Deterministic noise, so a rebuilt fixture is the same fixture.
fn noise(length: usize) -> Vec<f32> {
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let mut held = 0.0;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let white = (state >> 40) as f32 / (1u64 << 23) as f32 - 1.0;
            // One pole of low-pass: closer to a room than white noise is.
            held = 0.85 * held + 0.15 * white;
            held
        })
        .collect()
}

fn rms(samples: &[f32]) -> f32 {
    let energy: f32 = samples.iter().map(|s| s * s).sum();
    (energy / samples.len().max(1) as f32).sqrt()
}

/// `speech` with noise mixed in at `snr_db`.
pub fn with_noise(speech: &[f32], snr_db: f32) -> Vec<f32> {
    let noise = noise(speech.len());
    let gain = rms(speech) / rms(&noise).max(f32::EPSILON) / 10_f32.powf(snr_db / 20.0);
    speech
        .iter()
        .zip(&noise)
        .map(|(sample, noise)| sample + noise * gain)
        .collect()
}

/// The rate that brings a passage to `TARGET_SECONDS`, given how long it ran at the default.
fn fitted_rate(seconds: f32) -> u32 {
    (SPEAKING_RATE as f32 * seconds / TARGET_SECONDS).round() as u32
}

fn speak(text: &str, voice: &str, rate: u32, cache: &Path) -> anyhow::Result<Vec<f32>> {
    let path = spoken_path(cache, &format!("{voice}@{rate}"), text);
    if !path.exists() {
        say(voice, rate, text, &path)?;
    }
    read_wav(&path)
}

fn seconds(samples: &[f32]) -> f32 {
    samples.len() as f32 / SAMPLE_RATE_HZ as f32
}

pub fn build(script: &SpeechScript, recordings: &Path, out: &Path) -> anyhow::Result<Vec<Clip>> {
    let cache = out.join("spoken");
    fs::create_dir_all(&cache)?;
    let mut clips = Vec::new();
    for recording in &script.recording {
        let path = recordings.join(format!("{}.wav", recording.id));
        let length = seconds(&read_wav(&path)?);
        clips.push(Clip {
            id: recording.id.clone(),
            language: recording.language.clone(),
            source: Source::Recording,
            length: Length::Short,
            seconds: length,
            path,
            text: recording.text.clone(),
        });
    }
    for passage in &script.passage {
        let voices = script
            .voices
            .get(&passage.language)
            .with_context(|| format!("no voice speaks {}", passage.language))?;
        for (n, voice) in voices.iter().enumerate() {
            let mut speech = speak(&passage.text, voice, SPEAKING_RATE, &cache)?;
            if passage.length == Length::Short {
                let rate = fitted_rate(seconds(&speech));
                speech = speak(&passage.text, voice, rate, &cache)?;
            }
            for (source, tag) in [(Source::Clean, "clean"), (Source::Noisy, "noisy")] {
                let id = format!("{}-v{}-{tag}", passage.id, n + 1);
                let path = PathBuf::from(format!("{id}.wav"));
                let samples = match source {
                    Source::Noisy => with_noise(&speech, NOISE_SNR_DB),
                    _ => speech.clone(),
                };
                write_wav(&out.join(&path), &samples)?;
                clips.push(Clip {
                    id,
                    language: passage.language.clone(),
                    source,
                    length: passage.length,
                    seconds: seconds(&samples),
                    path,
                    text: passage.text.clone(),
                });
            }
        }
    }
    fs::write(
        out.join("clips.json"),
        serde_json::to_string_pretty(&clips)?,
    )?;
    Ok(clips)
}
