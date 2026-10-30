//! The pill's start, stop, lock and error cues (spec §1). They are synthesized from a few decaying
//! sine notes at the output device's own rate, so no sample files ship with the app and nothing
//! stays in memory between cues.

use std::f32::consts::TAU;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use wf_core::pill::Sound;

use crate::Error;

const PEAK: f32 = 0.28;
const ATTACK_S: f32 = 0.004;
const RELEASE_S: f32 = 0.012;
/// How far each note decays over its length, in time constants.
const DECAY: f32 = 3.5;
/// Second harmonic level, for a rounder tone than a bare sine.
const OVERTONE: f32 = 0.18;

struct Note {
    hz: f32,
    start_s: f32,
    length_s: f32,
}

const fn note(hz: f32, start_s: f32, length_s: f32) -> Note {
    Note {
        hz,
        start_s,
        length_s,
    }
}

const START: [Note; 2] = [note(622.0, 0.0, 0.08), note(932.0, 0.07, 0.11)];
const STOP: [Note; 2] = [note(932.0, 0.0, 0.08), note(622.0, 0.07, 0.11)];
const LOCK: [Note; 2] = [note(1245.0, 0.0, 0.05), note(1245.0, 0.075, 0.07)];
const ERROR: [Note; 2] = [note(294.0, 0.0, 0.1), note(233.0, 0.11, 0.17)];

fn score(sound: Sound) -> &'static [Note] {
    match sound {
        Sound::Start => &START,
        Sound::Stop => &STOP,
        Sound::Lock => &LOCK,
        Sound::Error => &ERROR,
    }
}

/// Renders a cue as mono samples at `sample_rate`.
pub fn synthesize(sound: Sound, sample_rate: u32) -> Vec<f32> {
    let notes = score(sound);
    let rate = sample_rate as f32;
    let seconds = notes
        .iter()
        .fold(0.0_f32, |end, n| end.max(n.start_s + n.length_s));
    let mut samples = vec![0.0; (seconds * rate).round() as usize];
    for note in notes {
        let first = (note.start_s * rate).round() as usize;
        let length = (note.length_s * rate).round() as usize;
        for (i, sample) in samples[first..].iter_mut().take(length).enumerate() {
            let t = i as f32 / rate;
            *sample += envelope(t, note.length_s) * tone(note.hz, t);
        }
    }
    let peak = samples.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
    samples.iter_mut().for_each(|s| *s *= PEAK / peak);
    samples
}

fn tone(hz: f32, t: f32) -> f32 {
    (TAU * hz * t).sin() + OVERTONE * (2.0 * TAU * hz * t).sin()
}

fn envelope(t: f32, length_s: f32) -> f32 {
    let attack = (t / ATTACK_S).min(1.0);
    let release = ((length_s - t) / RELEASE_S).clamp(0.0, 1.0);
    attack * release * (-DECAY * t / length_s).exp()
}

/// Plays a cue on the default output device without blocking. Failures are logged and otherwise
/// ignored: a missing cue must never get in the way of a dictation.
pub fn play(sound: Sound) {
    let spawned = std::thread::Builder::new()
        .name("pill-sound".into())
        .spawn(move || {
            if let Err(err) = play_blocking(sound) {
                tracing::warn!("could not play {sound:?}: {err}");
            }
        });
    if let Err(err) = spawned {
        tracing::warn!("could not start sound thread: {err}");
    }
}

/// Plays a cue and returns when it has finished.
pub fn play_blocking(sound: Sound) -> Result<(), Error> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| Error::DeviceUnavailable("default output".into()))?;
    let supported = device.default_output_config().map_err(backend)?;
    let config = supported.config();
    let samples = synthesize(sound, supported.sample_rate());
    let duration = samples.len() as f32 / supported.sample_rate() as f32;
    let channels = usize::from(supported.channels());
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &config, samples, channels),
        SampleFormat::I16 => build::<i16>(&device, &config, samples, channels),
        SampleFormat::I32 => build::<i32>(&device, &config, samples, channels),
        SampleFormat::U16 => build::<u16>(&device, &config, samples, channels),
        other => Err(Error::Backend(format!(
            "unsupported sample format {other:?}"
        ))),
    }?;
    stream.play().map_err(backend)?;
    std::thread::sleep(Duration::from_secs_f32(duration) + OUTPUT_LATENCY);
    Ok(())
}

/// Keeps the stream open for what is still in the device's buffer when the last sample is written.
const OUTPUT_LATENCY: Duration = Duration::from_millis(120);

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    samples: Vec<f32>,
    channels: usize,
) -> Result<cpal::Stream, Error>
where
    T: SizedSample + FromSample<f32>,
{
    let mut remaining = samples.into_iter();
    device
        .build_output_stream(
            *config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                for frame in data.chunks_mut(channels) {
                    let sample = T::from_sample(remaining.next().unwrap_or(0.0));
                    frame.fill(sample);
                }
            },
            |err: cpal::Error| tracing::warn!("output stream error: {err}"),
            None,
        )
        .map_err(backend)
}

fn backend(err: cpal::Error) -> Error {
    Error::Backend(err.to_string())
}
