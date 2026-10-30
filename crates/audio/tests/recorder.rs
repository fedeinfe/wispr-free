use std::time::Duration;

use wf_audio::Recorder;
use wf_core::audio::FRAME_SAMPLES;

/// Frames whose samples count up from `start`, so positions are recognisable in the clip.
fn counting_frames(start: usize, count: usize) -> Vec<Vec<f32>> {
    (0..count)
        .map(|f| {
            (0..FRAME_SAMPLES)
                .map(|i| (start + f * FRAME_SAMPLES + i) as f32)
                .collect()
        })
        .collect()
}

#[test]
fn a_clip_starts_300ms_before_start_was_called() {
    let mut recorder = Recorder::new();
    for frame in counting_frames(0, 25) {
        recorder.push(&frame); // one second of idle listening
    }

    recorder.start();
    for frame in counting_frames(25 * FRAME_SAMPLES, 10) {
        recorder.push(&frame);
    }
    let clip = recorder.stop();

    assert_eq!(clip.duration(), Duration::from_millis(300 + 400));
    // 300 ms = 4 800 samples before the 16 000th sample, then contiguous to the end.
    assert_eq!(clip.samples()[0], 16_000.0 - 4_800.0);
    assert!(clip.samples().windows(2).all(|w| w[1] == w[0] + 1.0));
}

#[test]
fn pre_roll_is_only_what_was_heard_when_listening_just_began() {
    let mut recorder = Recorder::new();
    recorder.push(&counting_frames(0, 1)[0]);

    recorder.start();
    let clip = recorder.stop();

    assert_eq!(clip.duration(), Duration::from_millis(40));
}

#[test]
fn audio_before_a_previous_dictation_does_not_leak_into_the_next() {
    let mut recorder = Recorder::new();
    for frame in counting_frames(0, 25) {
        recorder.push(&frame);
    }
    recorder.start();
    recorder.stop();

    recorder.start();
    let clip = recorder.stop();

    assert!(clip.samples().is_empty());
}

#[test]
fn frames_pushed_while_idle_are_not_recorded() {
    let mut recorder = Recorder::new();
    recorder.start();
    recorder.push(&counting_frames(0, 1)[0]);
    recorder.stop();
    for frame in counting_frames(FRAME_SAMPLES, 25) {
        recorder.push(&frame);
    }

    assert!(!recorder.is_recording());
    recorder.start();
    assert_eq!(recorder.stop().duration(), Duration::from_millis(300));
}
