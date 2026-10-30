use std::f32::consts::TAU;

use wf_audio::FrameConverter;
use wf_core::audio::{FRAME_SAMPLES, SAMPLE_RATE_HZ};

/// `seconds` of a sine at `freq` Hz, interleaved over `channels` identical channels.
fn sine(rate: u32, channels: usize, freq: f32, amplitude: f32, seconds: f32) -> Vec<f32> {
    let frames = (rate as f32 * seconds) as usize;
    (0..frames)
        .flat_map(|i| {
            let s = amplitude * (TAU * freq * i as f32 / rate as f32).sin();
            std::iter::repeat_n(s, channels)
        })
        .collect()
}

#[test]
fn stereo_48k_becomes_16k_mono_frames_of_40ms() {
    let mut converter = FrameConverter::new(48_000, 2).unwrap();

    let frames = converter.push(&sine(48_000, 2, 440.0, 0.5, 1.0));

    assert!(frames.iter().all(|f| f.len() == FRAME_SAMPLES));
    // One second is 25 frames; the resampler may hold back up to one frame of delay.
    assert!((24..=25).contains(&frames.len()), "{} frames", frames.len());
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

fn zero_crossings(samples: &[f32]) -> usize {
    samples
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count()
}

#[test]
fn a_tone_keeps_its_pitch_and_level_when_fed_in_odd_sized_chunks() {
    let mut converter = FrameConverter::new(44_100, 1).unwrap();
    let input = sine(44_100, 1, 1_000.0, 0.5, 2.0);

    let samples: Vec<f32> = input
        .chunks(373)
        .flat_map(|chunk| converter.push(chunk))
        .skip(1) // resampler warm-up
        .flatten()
        .collect();

    let seconds = samples.len() as f32 / SAMPLE_RATE_HZ as f32;
    let hz = zero_crossings(&samples) as f32 / 2.0 / seconds;
    assert!((hz - 1_000.0).abs() < 5.0, "{hz} Hz");
    let level = rms(&samples);
    assert!((level - 0.5 / 2f32.sqrt()).abs() < 0.01, "rms {level}");
}

#[test]
fn channels_are_averaged_into_mono() {
    let mut converter = FrameConverter::new(SAMPLE_RATE_HZ, 2).unwrap();
    let left_only: Vec<f32> = (0..FRAME_SAMPLES).flat_map(|_| [0.8, 0.0]).collect();

    let frames = converter.push(&left_only);

    assert_eq!(frames, vec![vec![0.4; FRAME_SAMPLES]]);
}
