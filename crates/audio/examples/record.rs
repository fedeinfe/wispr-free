//! Records from the microphone to a 16 kHz mono WAV and prints the level meter.
//!
//! ```sh
//! cargo run -p wf-audio --example record -- out.wav            # 5 s, system default mic
//! cargo run -p wf-audio --example record -- out.wav 10 <id>... # 10 s, ranked device ids
//! cargo run -p wf-audio --example record -- --list             # ranked device list
//! ```
//!
//! Unplug the mic while it runs to watch capture fall back to the next device.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::{Duration, Instant};

use wf_audio::{
    AudioBackend, Capture, CaptureConfig, CaptureEvent, CpalBackend, LevelMeter, Recorder,
    rank_devices,
};
use wf_core::audio::{AudioClip, SAMPLE_RATE_HZ};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--list") {
        let backend = CpalBackend::new();
        let default = backend.default_device();
        for device in rank_devices(&backend.devices(), &[], default.as_deref()) {
            println!("{}\t{}", device.id, device.name);
        }
        return Ok(());
    }
    let path = args
        .first()
        .ok_or("usage: record <out.wav> [seconds] [device-id...]")?;
    let seconds: u64 = args.get(1).map_or(Ok(5), |s| s.parse())?;
    let ranking = args.iter().skip(2).cloned().collect();

    let capture = Capture::start(CaptureConfig {
        ranking,
        ..CaptureConfig::default()
    })?;
    let mut recorder = Recorder::new();
    let mut meter = LevelMeter::new();
    recorder.start();

    let deadline = Instant::now() + Duration::from_secs(seconds);
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match capture.events().recv_timeout(left) {
            Ok(CaptureEvent::Device(Some(device))) => {
                meter.reset();
                println!("device: {} ({})", device.name, device.id);
            }
            Ok(CaptureEvent::Device(None)) => println!("device: none (waiting for a mic)"),
            Ok(CaptureEvent::Frame(frame)) => {
                recorder.push(&frame);
                if let Some(level) = meter.push(&frame) {
                    let bars = (level * 40.0).round() as usize;
                    println!("level {level:.2} |{:<40}|", "#".repeat(bars));
                }
            }
            Err(_) => break,
        }
    }

    let clip = recorder.stop();
    write_wav(path, &clip)?;
    println!("wrote {:.2} s to {path}", clip.duration().as_secs_f64());
    Ok(())
}

/// 16-bit PCM, mono, pipeline rate.
fn write_wav(path: &str, clip: &AudioClip) -> std::io::Result<()> {
    let data_len = (clip.samples().len() * 2) as u32;
    let mut out = BufWriter::new(File::create(path)?);
    out.write_all(b"RIFF")?;
    out.write_all(&(36 + data_len).to_le_bytes())?;
    out.write_all(b"WAVEfmt ")?;
    out.write_all(&16u32.to_le_bytes())?; // fmt chunk size
    out.write_all(&1u16.to_le_bytes())?; // PCM
    out.write_all(&1u16.to_le_bytes())?; // mono
    out.write_all(&SAMPLE_RATE_HZ.to_le_bytes())?;
    out.write_all(&(SAMPLE_RATE_HZ * 2).to_le_bytes())?; // byte rate
    out.write_all(&2u16.to_le_bytes())?; // block align
    out.write_all(&16u16.to_le_bytes())?; // bits per sample
    out.write_all(b"data")?;
    out.write_all(&data_len.to_le_bytes())?;
    for &s in clip.samples() {
        let pcm = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
        out.write_all(&pcm.to_le_bytes())?;
    }
    out.flush()
}
