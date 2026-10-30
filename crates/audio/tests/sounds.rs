use wf_audio::sounds::synthesize;
use wf_core::pill::Sound;

const ALL: [Sound; 4] = [Sound::Start, Sound::Stop, Sound::Lock, Sound::Error];
const RATE: u32 = 48_000;

/// Dominant pitch of a stretch of audio, from its upward zero crossings.
fn pitch_hz(samples: &[f32]) -> f32 {
    let crossings = samples
        .windows(2)
        .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
        .count();
    crossings as f32 * RATE as f32 / samples.len() as f32
}

fn opening_and_closing_pitch(sound: Sound) -> (f32, f32) {
    let samples = synthesize(sound, RATE);
    let third = samples.len() / 3;
    (pitch_hz(&samples[..third]), pitch_hz(&samples[2 * third..]))
}

#[test]
fn every_sound_is_short() {
    for sound in ALL {
        let seconds = synthesize(sound, RATE).len() as f32 / RATE as f32;
        assert!((0.05..=0.4).contains(&seconds), "{sound:?}: {seconds} s");
    }
}

#[test]
fn duration_does_not_depend_on_the_device_rate() {
    for sound in ALL {
        let at_48k = synthesize(sound, 48_000).len() as f32 / 48_000.0;
        let at_44k = synthesize(sound, 44_100).len() as f32 / 44_100.0;
        assert!((at_48k - at_44k).abs() < 0.001, "{sound:?}");
    }
}

#[test]
fn every_sound_is_audible_but_gentle() {
    for sound in ALL {
        let peak = synthesize(sound, RATE)
            .iter()
            .fold(0.0_f32, |peak, s| peak.max(s.abs()));
        assert!((0.1..=0.4).contains(&peak), "{sound:?}: peak {peak}");
    }
}

#[test]
fn every_sound_fades_in_and_out_without_a_click() {
    for sound in ALL {
        let samples = synthesize(sound, RATE);
        let edge = RATE as usize / 2_000; // 0.5 ms
        let start_peak = samples[..edge].iter().fold(0.0_f32, |p, s| p.max(s.abs()));
        let end_peak = samples[samples.len() - edge..]
            .iter()
            .fold(0.0_f32, |p, s| p.max(s.abs()));
        assert!(start_peak < 0.05, "{sound:?} starts at {start_peak}");
        assert!(end_peak < 0.01, "{sound:?} ends at {end_peak}");
    }
}

#[test]
fn start_rises_and_stop_falls() {
    let (from, to) = opening_and_closing_pitch(Sound::Start);
    assert!(to > from * 1.2, "start: {from} -> {to}");
    let (from, to) = opening_and_closing_pitch(Sound::Stop);
    assert!(to < from / 1.2, "stop: {from} -> {to}");
}

#[test]
fn the_error_sound_is_the_lowest() {
    let error = opening_and_closing_pitch(Sound::Error).0;
    for sound in [Sound::Start, Sound::Stop, Sound::Lock] {
        let (from, to) = opening_and_closing_pitch(sound);
        assert!(error < from.min(to), "{sound:?}: {error} vs {from}/{to}");
    }
}

#[test]
fn the_four_sounds_differ() {
    for (i, a) in ALL.iter().enumerate() {
        for b in &ALL[i + 1..] {
            assert_ne!(synthesize(*a, RATE), synthesize(*b, RATE), "{a:?} {b:?}");
        }
    }
}
