//! Times one ASR model over the clips from `speech_fixture` and scores its word error rate. One
//! model per run, so a benchmark never holds two in memory.

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::Context as _;
use wf_core::audio::AudioClip;
use wf_models::{Catalog, Engine};
use wf_pipeline::sherpa::SherpaAsr;
use wf_pipeline::whisper::WhisperAsr;
use wf_pipeline::{Asr, AsrOptions};

use crate::asr_score::{BenchReport, ClipResult, Mode, render};
use crate::meeting_eval::word_edits;
use crate::meeting_fixture::read_wav;
use crate::speech_fixture::{self, Clip};

#[derive(Debug, Clone)]
pub struct Bench {
    pub fixture: PathBuf,
    pub models: PathBuf,
    pub model: String,
    /// The JSON report with every transcript and time.
    pub out: PathBuf,
    pub runs: usize,
    pub filter: Option<String>,
    pub mode: Option<String>,
    /// CPU only, with this many threads. `None` runs the way the app does.
    pub cpu_threads: Option<usize>,
}

fn engine(bench: &Bench) -> anyhow::Result<(Box<dyn Asr>, Vec<Mode>)> {
    let catalog = Catalog::bundled();
    let model = catalog
        .model(&bench.model)
        .with_context(|| format!("{} isn't in the catalog", bench.model))?;
    let dir = bench.models.join(&model.id);
    Ok(match model.engine {
        Engine::WhisperCpp => {
            let file = dir.join(&model.files[0].name);
            let asr = match bench.cpu_threads {
                Some(threads) => WhisperAsr::with_threads(&file, false, threads)?,
                None => WhisperAsr::new(&file)?,
            };
            let multilingual = model.languages.iter().any(|language| language == "*");
            let modes = if multilingual {
                vec![Mode::Auto, Mode::Pinned]
            } else {
                vec![Mode::Auto]
            };
            (Box::new(asr), modes)
        }
        Engine::SherpaOnnx => {
            let asr = match bench.cpu_threads {
                Some(threads) => SherpaAsr::with_threads(&dir, threads)?,
                None => SherpaAsr::new(&dir)?,
            };
            // Parakeet has nothing to pin: it follows the speaker.
            (Box::new(asr), vec![Mode::Auto])
        }
        Engine::LlamaCpp => anyhow::bail!("{} isn't an ASR model", model.id),
    })
}

fn wanted(mode: Mode, chosen: Option<&str>) -> bool {
    chosen.is_none_or(|chosen| format!("{mode:?}").eq_ignore_ascii_case(chosen))
}

pub fn run(bench: &Bench) -> anyhow::Result<String> {
    let clips: Vec<Clip> = speech_fixture::clips(&bench.fixture)?
        .into_iter()
        .filter(|clip| {
            bench
                .filter
                .as_deref()
                .is_none_or(|text| clip.id.contains(text))
        })
        .collect();
    anyhow::ensure!(!clips.is_empty(), "no clip matches");
    let audio = clips
        .iter()
        .map(|clip| {
            let samples = read_wav(&bench.fixture.join(&clip.path))?;
            Ok(AudioClip::new(samples))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    let started = Instant::now();
    let (mut asr, modes) = engine(bench)?;
    let load_seconds = started.elapsed().as_secs_f64();

    // The first run compiles GPU shaders and pages the model in, which no later one pays.
    asr.transcribe(&audio[0], &AsrOptions::default())?;

    let mut results = Vec::new();
    for mode in modes
        .into_iter()
        .filter(|&m| wanted(m, bench.mode.as_deref()))
    {
        for (clip, samples) in clips.iter().zip(&audio) {
            let options = AsrOptions {
                language: (mode == Mode::Pinned).then(|| clip.language.clone()),
                ..AsrOptions::default()
            };
            let mut times = Vec::with_capacity(bench.runs);
            let mut varied = false;
            let mut first = None;
            for _ in 0..bench.runs {
                let started = Instant::now();
                let transcript = asr.transcribe(samples, &options)?;
                times.push(started.elapsed().as_secs_f64());
                match &first {
                    None => first = Some(transcript),
                    Some(earlier) => varied |= earlier.text != transcript.text,
                }
            }
            let transcript = first.context("set --runs to at least 1")?;
            let (edits, words) = word_edits(&clip.text, &transcript.text);
            results.push(ClipResult {
                id: clip.id.clone(),
                language: clip.language.clone(),
                source: clip.source,
                length: clip.length,
                seconds: clip.seconds,
                mode,
                times,
                detected: transcript.language,
                text: transcript.text,
                varied,
                edits,
                words,
            });
        }
    }

    let report = BenchReport {
        model: bench.model.clone(),
        cpu_threads: bench.cpu_threads,
        load_seconds,
        clips: results,
    };
    fs::write(&bench.out, serde_json::to_string_pretty(&report)?)?;
    Ok(render(&report))
}
