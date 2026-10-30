//! Loads the committed WAV fixtures without an engine's reader, so every feature can use them.

use wf_core::audio::AudioClip;

/// Reads a 16-bit mono fixture, walking the RIFF chunks to find the samples.
pub fn fixture(name: &str) -> AudioClip {
    let path = format!("{}/tests/fixtures/{name}.wav", env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(&path).expect(&path);
    let mut at = 12;
    loop {
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        if &bytes[at..at + 4] == b"data" {
            let samples = bytes[at + 8..at + 8 + size]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&pair| f32::from(i16::from_le_bytes(pair)) / 32_768.0)
                .collect();
            return AudioClip::new(samples);
        }
        at += 8 + size + size % 2;
    }
}
