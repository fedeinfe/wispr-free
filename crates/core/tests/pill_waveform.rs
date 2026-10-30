use std::time::Duration;

use wf_core::pill::waveform::{BARS, Waveform};

const FRAME: Duration = Duration::from_millis(16);

/// Runs long enough for smoothing to settle, then returns the average height of each bar, which
/// cancels the wobble.
fn settled_mean(level: f32) -> [f32; BARS] {
    let mut waveform = Waveform::new();
    waveform.set_level(level);
    for _ in 0..200 {
        waveform.advance(FRAME);
    }
    let mut sum = [0.0; BARS];
    let frames = 4000;
    for _ in 0..frames {
        for (total, bar) in sum.iter_mut().zip(waveform.advance(FRAME)) {
            *total += bar;
        }
    }
    sum.map(|total| total / frames as f32)
}

#[test]
fn there_are_ten_bars() {
    assert_eq!(Waveform::new().advance(FRAME).len(), 10);
}

#[test]
fn silence_leaves_low_nearly_flat_bars() {
    let bars = settled_mean(0.0);
    assert!(bars.iter().all(|&b| b > 0.0 && b < 0.2), "{bars:?}");
}

#[test]
fn loud_input_gives_a_bell_shape_peaking_in_the_middle() {
    let bars = settled_mean(1.0);
    for i in 0..BARS / 2 - 1 {
        assert!(bars[i] < bars[i + 1], "not rising at {i}: {bars:?}");
    }
    for i in 0..BARS / 2 {
        let (left, right) = (bars[i], bars[BARS - 1 - i]);
        assert!((left - right).abs() < 0.02, "not symmetric: {bars:?}");
    }
    assert!(bars[0] < 0.5, "edges should stay low: {bars:?}");
    assert!(bars[BARS / 2] > 0.8, "middle should be tall: {bars:?}");
}

#[test]
fn bars_stay_within_their_full_height_at_every_moment() {
    let mut waveform = Waveform::new();
    waveform.set_level(1.0);
    for _ in 0..2000 {
        let bars = waveform.advance(FRAME);
        assert!(bars.iter().all(|&b| (0.0..=1.0).contains(&b)), "{bars:?}");
    }
}

#[test]
fn the_wobble_travels_along_the_bars() {
    // Bar 4's excursion from its mean reappears at bar 5 a little later.
    let mut waveform = Waveform::new();
    waveform.set_level(0.8);
    for _ in 0..200 {
        waveform.advance(FRAME);
    }
    let frames: Vec<[f32; BARS]> = (0..600).map(|_| waveform.advance(FRAME)).collect();
    let series = |bar: usize| -> Vec<f32> {
        let mean = frames.iter().map(|f| f[bar]).sum::<f32>() / frames.len() as f32;
        frames.iter().map(|f| f[bar] - mean).collect()
    };
    let (a, b) = (series(4), series(5));
    let correlation =
        |lag: usize| -> f32 { a.iter().zip(&b[lag..]).map(|(x, y)| x * y).sum::<f32>() };
    let best_lag = (0..120)
        .max_by(|&x, &y| correlation(x).total_cmp(&correlation(y)))
        .unwrap();
    assert!(best_lag > 0, "the two bars move in phase");
    assert!(a.iter().any(|&x| x.abs() > 0.01), "no visible wobble");
}

#[test]
fn level_changes_are_smoothed_over_several_frames() {
    let mut waveform = Waveform::new();
    let before = waveform.advance(FRAME)[5];
    waveform.set_level(1.0);
    let first = waveform.advance(FRAME)[5];
    for _ in 0..60 {
        waveform.advance(FRAME);
    }
    let settled = waveform.advance(FRAME)[5];
    assert!(first > before, "the jump should start right away");
    assert!(first - before < 0.5 * (settled - before), "jumped too fast");
}

#[test]
fn smoothing_does_not_depend_on_the_frame_rate() {
    let run = |frames: u32, dt: Duration| {
        let mut waveform = Waveform::new();
        waveform.set_level(1.0);
        (0..frames).fold(0.0, |_, _| waveform.advance(dt)[5])
    };
    let fast = run(6, Duration::from_millis(16));
    let slow = run(2, Duration::from_millis(48));
    assert!((fast - slow).abs() < 0.01, "{fast} vs {slow}");
}

#[test]
fn reset_returns_to_silence_immediately() {
    let mut waveform = Waveform::new();
    waveform.set_level(1.0);
    for _ in 0..100 {
        waveform.advance(FRAME);
    }
    waveform.reset();
    let bars = waveform.advance(FRAME);
    assert!(bars.iter().all(|&b| b < 0.2), "{bars:?}");
}
