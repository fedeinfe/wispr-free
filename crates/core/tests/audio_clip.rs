use std::time::Duration;

use wf_core::audio::{AudioClip, FRAME_SAMPLES, SAMPLE_RATE_HZ};

#[test]
fn pipeline_audio_is_16khz_mono_in_40ms_frames() {
    assert_eq!(SAMPLE_RATE_HZ, 16_000);
    assert_eq!(FRAME_SAMPLES, 640);
}

#[test]
fn clip_duration_follows_the_pipeline_sample_rate() {
    let clip = AudioClip::new(vec![0.0; 24_000]);

    assert_eq!(clip.duration(), Duration::from_millis(1_500));
}
