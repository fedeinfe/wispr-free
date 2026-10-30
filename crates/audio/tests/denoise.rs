//! RNNoise on real speech plus synthetic noise.

use wf_audio::Denoiser;
use wf_core::audio::{FRAME_SAMPLES, SAMPLE_RATE_HZ};

fn speech() -> Vec<f32> {
    let path = format!(
        "{}/../pipeline/tests/fixtures/speech_en.wav",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(path).unwrap();
    bytes[44..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| f32::from(i16::from_le_bytes(pair)) / 32_768.0)
        .collect()
}

/// A little xorshift, so the noise is the same in every run.
fn noise(len: usize, level: f32) -> Vec<f32> {
    let mut state = 0x2545_f491_u32;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state as f32 / u32::MAX as f32 - 0.5) * 2.0 * level
        })
        .collect()
}

fn rms(samples: &[f32]) -> f32 {
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len().max(1) as f32).sqrt()
}

fn denoise(samples: &[f32]) -> Vec<f32> {
    let mut denoiser = Denoiser::new().unwrap();
    let mut out = Vec::new();
    for frame in samples.chunks(FRAME_SAMPLES) {
        out.extend(denoiser.process(frame).into_iter().flatten());
    }
    out
}

#[test]
fn steady_noise_drops_and_speech_stays() {
    let seconds = SAMPLE_RATE_HZ as usize;
    let speech = speech();
    let lead_in = seconds * 4;
    let hiss = noise(lead_in + speech.len(), 0.02);
    let mut noisy = hiss.clone();
    for (sample, spoken) in noisy[lead_in..].iter_mut().zip(&speech) {
        *sample += spoken;
    }

    let cleaned = denoise(&noisy);

    let settled_noise = rms(&cleaned[lead_in - seconds..lead_in - seconds / 4]);
    assert!(
        settled_noise < rms(&hiss) / 4.0,
        "noise went from {} to {settled_noise}",
        rms(&hiss)
    );
    let voiced = rms(&cleaned[lead_in + seconds / 2..cleaned.len() - seconds / 2]);
    let original = rms(&speech[seconds / 2..speech.len() - seconds / 2]);
    assert!(
        voiced > original / 2.0,
        "speech fell from {original} to {voiced}"
    );
}

#[test]
fn output_comes_in_pipeline_frames_and_keeps_pace_with_the_input() {
    let input = noise(FRAME_SAMPLES * 50, 0.1);
    let mut denoiser = Denoiser::new().unwrap();
    let mut out = 0;

    for frame in input.chunks(FRAME_SAMPLES) {
        for cleaned in denoiser.process(frame) {
            assert_eq!(cleaned.len(), FRAME_SAMPLES);
            out += cleaned.len();
        }
    }

    assert!(out + FRAME_SAMPLES * 3 >= input.len(), "{out}");
    assert!(out <= input.len());
}
