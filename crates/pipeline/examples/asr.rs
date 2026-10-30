//! Transcribes WAV files through the ASR engines and reports the real-time factor: processing
//! time over audio time, so below 1 is faster than speech. Files must be 16 kHz mono.
//!
//! ```sh
//! cargo run --release -p wf-pipeline --features whisper,sherpa,silero,metal --example asr -- \
//!     --whisper ggml-small-q8_0.bin --parakeet parakeet-tdt-0.6b-v3-int8 a.wav b.wav
//! ```
//!
//! `--language it` fixes the language and `--bias word` (repeatable) biases the decoder.
//! `--live silero_vad.onnx` also streams each file in capture-sized frames, as a meeting would.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Instant;

use wf_core::audio::{AudioClip, FRAME_SAMPLES, SAMPLE_RATE_HZ};
use wf_pipeline::sherpa::SherpaAsr;
use wf_pipeline::silero::SileroVad;
use wf_pipeline::vad::{LiveVad, VadOptions};
use wf_pipeline::whisper::WhisperAsr;
use wf_pipeline::worker::AsrWorker;
use wf_pipeline::{Asr, AsrOptions};

#[derive(Default)]
struct Args {
    whisper: Option<PathBuf>,
    parakeet: Option<PathBuf>,
    live: Option<PathBuf>,
    options: AsrOptions,
    wavs: Vec<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut parsed = Args::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--whisper" => parsed.whisper = Some(value()?.into()),
            "--parakeet" => parsed.parakeet = Some(value()?.into()),
            "--live" => parsed.live = Some(value()?.into()),
            "--language" => parsed.options.language = Some(value()?),
            "--bias" => parsed.options.bias_words.push(value()?),
            _ if arg.starts_with("--") => return Err(format!("unknown option {arg}")),
            _ => parsed.wavs.push(arg.into()),
        }
    }
    if parsed.wavs.is_empty() || (parsed.whisper.is_none() && parsed.parakeet.is_none()) {
        return Err(
            "usage: asr [--whisper MODEL] [--parakeet DIR] [--language CODE] \
                    [--bias WORD]... [--live SILERO_MODEL] WAV..."
                .into(),
        );
    }
    Ok(parsed)
}

fn read_wav(path: &Path) -> Result<AudioClip, String> {
    let name = path.display();
    let wave = sherpa_onnx::Wave::read(&path.to_string_lossy())
        .ok_or_else(|| format!("{name}: not a readable WAV"))?;
    if wave.sample_rate() != SAMPLE_RATE_HZ as i32 {
        return Err(format!("{name}: {} Hz, needs 16 kHz", wave.sample_rate()));
    }
    Ok(AudioClip::new(wave.samples().to_vec()))
}

fn real_time_factor(processing: f64, clip: &AudioClip) -> f64 {
    processing / clip.duration().as_secs_f64()
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into(),
    )
}

fn transcribe_files(
    engine: &str,
    asr: &mut dyn Asr,
    files: &[(String, AudioClip)],
    options: &AsrOptions,
) -> Result<(), Box<dyn Error>> {
    // The first run compiles GPU shaders and pages the model in, which no later one pays.
    asr.transcribe(&files[0].1, options)?;
    for (name, clip) in files {
        let started = Instant::now();
        let transcript = asr.transcribe(clip, options)?;
        let seconds = started.elapsed().as_secs_f64();
        println!(
            "{engine:8} {name:20} {:7.2} s audio {seconds:7.3} s  RTF {:.3}  [{}] {}",
            clip.duration().as_secs_f64(),
            real_time_factor(seconds, clip),
            transcript.language.as_deref().unwrap_or("-"),
            transcript.text,
        );
    }
    Ok(())
}

/// Feeds a file through the live path: VAD inline, ASR on the worker.
fn stream(
    engine: &str,
    mut asr: Box<dyn Asr>,
    vad: &mut SileroVad,
    (name, clip): &(String, AudioClip),
    options: &AsrOptions,
) -> Result<Box<dyn Asr>, Box<dyn Error>> {
    let started = Instant::now();
    let worker = AsrWorker::spawn(asr, options.clone());
    let mut live = LiveVad::new(vad, VadOptions::default());
    for frame in clip.samples().chunks(FRAME_SAMPLES) {
        for chunk in live.push(frame)?.chunks {
            worker.submit((), chunk)?;
        }
    }
    for chunk in live.finish()? {
        worker.submit((), chunk)?;
    }
    let results;
    (results, asr) = worker.finish()?;
    let seconds = started.elapsed().as_secs_f64();
    println!(
        "{engine:8} {name:20} {:7.2} s live  {seconds:7.3} s  RTF {:.3}",
        clip.duration().as_secs_f64(),
        real_time_factor(seconds, clip),
    );
    for result in results {
        let chunk = result?;
        let (start, end) = (chunk.start.as_secs(), chunk.end.as_secs());
        println!(
            "  [{:02}:{:02}-{:02}:{:02}] {}",
            start / 60,
            start % 60,
            end / 60,
            end % 60,
            chunk.transcript.text
        );
    }
    Ok(asr)
}

fn run(
    engine: &str,
    load: impl FnOnce() -> Result<Box<dyn Asr>, wf_pipeline::PipelineError>,
    files: &[(String, AudioClip)],
    args: &Args,
) -> Result<(), Box<dyn Error>> {
    let started = Instant::now();
    let mut asr = load()?;
    println!(
        "{engine:8} loaded in {:.2} s",
        started.elapsed().as_secs_f64()
    );
    transcribe_files(engine, asr.as_mut(), files, &args.options)?;
    if let Some(model) = &args.live {
        let mut vad = SileroVad::new(model)?;
        for file in files {
            asr = stream(engine, asr, &mut vad, file, &args.options)?;
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let mut files = Vec::new();
    for path in &args.wavs {
        files.push((file_name(path), read_wav(path)?));
    }
    if let Some(model) = &args.whisper {
        let load = || Ok(Box::new(WhisperAsr::new(model)?) as Box<dyn Asr>);
        run("whisper", load, &files, &args)?;
    }
    if let Some(dir) = &args.parakeet {
        let load = || Ok(Box::new(SherpaAsr::new(dir)?) as Box<dyn Asr>);
        run("parakeet", load, &files, &args)?;
    }
    Ok(())
}
