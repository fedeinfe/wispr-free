//! Builds the 30-minute meeting's audio from `eval/meeting.toml` with macOS `say`: one voice per
//! person, `you.wav` for the mic and `others.wav` for the system channel, and `truth.json` with
//! who said what when. Turns are cached by content, so a second run takes seconds.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, bail};
use wf_core::audio::SAMPLE_RATE_HZ;

use crate::meeting_eval::{MeetingScript, PlacedTurn, Truth};

/// A little under the default, which gets the script to half an hour.
const SPEAKING_RATE: &str = "165";
const TARGET_MS: u64 = 30 * 60 * 1_000;
/// Every twelfth turn cuts into the one before, as people do.
const INTERRUPT_EVERY: usize = 12;
const INTERRUPT_MS: u64 = 350;
/// About −66 dBFS on the mic, so its pauses aren't digital silence. System audio has that.
const ROOM_NOISE: f32 = 0.0005;

fn wav_path(dir: &Path, voice: &str, text: &str) -> PathBuf {
    let key = text
        .bytes()
        .chain(voice.bytes())
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });
    dir.join(format!("{key:016x}.wav"))
}

fn say(voice: &str, text: &str, path: &Path) -> anyhow::Result<()> {
    let status = Command::new("say")
        .args(["-v", voice, "-r", SPEAKING_RATE])
        .args(["--file-format=WAVE", "--data-format=LEI16@16000", "-o"])
        .arg(path)
        .arg(text)
        .status()
        .context("running macOS `say`")?;
    if !status.success() {
        bail!("`say` failed for voice {voice}");
    }
    Ok(())
}

/// Reads 16-bit mono PCM, walking the RIFF chunks.
pub fn read_wav(path: &Path) -> anyhow::Result<Vec<f32>> {
    let bytes = fs::read(path).with_context(|| path.display().to_string())?;
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into()?) as usize;
        if &bytes[at..at + 4] == b"data" {
            let end = (at + 8 + size).min(bytes.len());
            let (samples, _) = bytes[at + 8..end].as_chunks::<2>();
            return Ok(samples
                .iter()
                .map(|&sample| f32::from(i16::from_le_bytes(sample)) / 32_768.0)
                .collect());
        }
        at += 8 + size + size % 2;
    }
    bail!("{} has no audio", path.display())
}

fn write_wav(path: &Path, samples: &[f32]) -> anyhow::Result<()> {
    let data_len = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&SAMPLE_RATE_HZ.to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE_HZ * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * 32_767.0) as i16;
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    Ok(fs::write(path, bytes)?)
}

/// Deterministic, so a rebuilt fixture is the same fixture.
fn wobble(n: usize) -> u64 {
    (n as u64).wrapping_mul(2_654_435_761) >> 16 & 0xffff
}

fn to_samples(ms: u64) -> usize {
    (ms * u64::from(SAMPLE_RATE_HZ) / 1_000) as usize
}

fn to_ms(samples: usize) -> u64 {
    samples as u64 * 1_000 / u64::from(SAMPLE_RATE_HZ)
}

pub fn build(script: &MeetingScript, out: &Path) -> anyhow::Result<Truth> {
    let cache = out.join("turns");
    fs::create_dir_all(&cache)?;
    let mut clips = Vec::with_capacity(script.turns().len());
    for (who, text) in script.turns() {
        let voice = script
            .voices
            .get(who)
            .context("a turn's speaker has no voice")?;
        let path = wav_path(&cache, voice, text);
        if !path.exists() {
            say(voice, text, &path)?;
        }
        clips.push(read_wav(&path)?);
    }

    // Stretch the pauses so the whole thing lands on the target length.
    let speech: u64 = clips.iter().map(|clip| to_ms(clip.len())).sum();
    let pause = TARGET_MS.saturating_sub(speech) / clips.len() as u64;
    println!("speech {} s, pause {pause} ms on average", speech / 1_000);

    let mut you = vec![0.0; to_samples(TARGET_MS)];
    let mut others = vec![0.0; to_samples(TARGET_MS)];
    let mut turns = Vec::with_capacity(clips.len());
    let mut cursor: u64 = 500;
    for (n, ((who, text), clip)) in script.turns().iter().zip(&clips).enumerate() {
        let start_ms = if n % INTERRUPT_EVERY == INTERRUPT_EVERY - 1 {
            cursor.saturating_sub(INTERRUPT_MS)
        } else {
            cursor + pause / 2 + pause * wobble(n) / 0xffff
        };
        let start = to_samples(start_ms);
        let channel = if *who == script.you {
            &mut you
        } else {
            &mut others
        };
        if start + clip.len() > channel.len() {
            channel.resize(start + clip.len(), 0.0);
        }
        for (slot, sample) in channel[start..].iter_mut().zip(clip) {
            *slot += sample;
        }
        let end_ms = start_ms + to_ms(clip.len());
        turns.push(PlacedTurn {
            who: who.clone(),
            start_ms,
            end_ms,
            text: text.clone(),
        });
        cursor = end_ms;
    }

    let length = you.len().max(others.len());
    you.resize(length, 0.0);
    others.resize(length, 0.0);
    for (n, sample) in you.iter_mut().enumerate() {
        *sample += ROOM_NOISE * (wobble(n) as f32 / 32_768.0 - 1.0);
    }
    write_wav(&out.join("you.wav"), &you)?;
    write_wav(&out.join("others.wav"), &others)?;
    let truth = Truth {
        length_ms: to_ms(length),
        turns,
    };
    fs::write(out.join("truth.json"), serde_json::to_vec_pretty(&truth)?)?;
    Ok(truth)
}
