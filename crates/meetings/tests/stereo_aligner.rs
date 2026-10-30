//! Lining up two capture streams that run on different clocks.

use wf_core::audio::FRAME_SAMPLES;
use wf_meetings::{Channel, StereoAligner, StereoFrame};

fn mic(aligner: &mut StereoAligner, value: f32) -> Vec<StereoFrame> {
    aligner.push(Channel::You, vec![value; FRAME_SAMPLES])
}

fn system(aligner: &mut StereoAligner, value: f32) -> Vec<StereoFrame> {
    aligner.push(Channel::Others, vec![value; FRAME_SAMPLES])
}

fn levels(frames: &[StereoFrame]) -> Vec<(f32, f32)> {
    frames.iter().map(|f| (f.you[0], f.others[0])).collect()
}

#[test]
fn frames_from_both_channels_are_paired_in_order() {
    let mut aligner = StereoAligner::new();

    assert!(mic(&mut aligner, 0.1).is_empty());
    assert_eq!(levels(&system(&mut aligner, 0.2)), [(0.1, 0.2)]);
    assert!(system(&mut aligner, 0.4).is_empty());
    assert_eq!(levels(&mic(&mut aligner, 0.3)), [(0.3, 0.4)]);
}

#[test]
fn a_silent_system_channel_does_not_hold_the_mic_back() {
    // ScreenCaptureKit sends nothing at all while nothing plays.
    let mut aligner = StereoAligner::new();

    let written: Vec<StereoFrame> = (0..100).flat_map(|_| mic(&mut aligner, 0.5)).collect();

    let delayed = 100 - written.len();
    assert!(delayed * 40 <= 200, "mic held back by {} ms", delayed * 40);
    assert!(written.iter().all(|f| f.others.iter().all(|&s| s == 0.0)));
    assert!(written.iter().all(|f| f.you.iter().all(|&s| s == 0.5)));
}

#[test]
fn a_missing_mic_does_not_hold_system_audio_back() {
    let mut aligner = StereoAligner::new();

    let written: Vec<StereoFrame> = (0..100).flat_map(|_| system(&mut aligner, 0.5)).collect();

    assert!(written.len() >= 94);
    assert!(written.iter().all(|f| f.you.iter().all(|&s| s == 0.0)));
}

#[test]
fn audio_after_a_gap_lands_at_its_real_time() {
    let mut aligner = StereoAligner::new();
    let mut written = Vec::new();
    for _ in 0..50 {
        written.extend(mic(&mut aligner, 0.1));
        written.extend(system(&mut aligner, 0.2));
    }
    // The system stream drops out for 2 s, then both run again.
    for _ in 0..50 {
        written.extend(mic(&mut aligner, 0.1));
    }
    for _ in 0..50 {
        written.extend(system(&mut aligner, 0.2));
        written.extend(mic(&mut aligner, 0.1));
    }
    written.extend(aligner.drain());

    assert_eq!(written.len(), 150, "the timeline keeps its real length");
    let filled_with_silence = written.iter().filter(|f| f.others[0] == 0.0).count();
    assert!(
        (45..=50).contains(&filled_with_silence),
        "{filled_with_silence} frames"
    );
}

#[test]
fn a_slow_clock_stays_within_the_skew_limit_for_an_hour() {
    // 0.1% drift between the sound cards: 3.6 s an hour.
    let mut aligner = StereoAligner::new();
    let mut written = 0;
    let frames_per_hour = 90_000;
    for n in 0..frames_per_hour {
        written += mic(&mut aligner, 0.1).len();
        if n % 1000 != 0 {
            written += system(&mut aligner, 0.2).len();
        }
    }

    let behind = frames_per_hour - written;
    assert!(behind * 40 <= 200, "{} ms behind", behind * 40);
}

#[test]
fn drain_pads_whatever_is_left() {
    let mut aligner = StereoAligner::new();
    mic(&mut aligner, 0.1);
    mic(&mut aligner, 0.2);

    let frames = aligner.drain();

    assert_eq!(levels(&frames), [(0.1, 0.0), (0.2, 0.0)]);
    assert!(aligner.drain().is_empty());
}

#[test]
fn short_input_waits_for_a_whole_frame() {
    let mut aligner = StereoAligner::new();

    assert!(aligner.push(Channel::You, vec![0.1; 100]).is_empty());
    assert!(
        aligner
            .push(Channel::Others, vec![0.2; FRAME_SAMPLES])
            .is_empty()
    );
    let frames = aligner.push(Channel::You, vec![0.1; FRAME_SAMPLES - 100]);

    assert_eq!(levels(&frames), [(0.1, 0.2)]);
}
