use wf_audio::LevelMeter;
use wf_core::audio::FRAME_SAMPLES;

/// A ±amplitude square wave: its RMS level is exactly `db` dBFS.
fn frames_at(db: f32, count: usize) -> Vec<Vec<f32>> {
    let amplitude = 10f32.powf(db / 20.0);
    let frame: Vec<f32> = (0..FRAME_SAMPLES)
        .map(|i| if i % 2 == 0 { amplitude } else { -amplitude })
        .collect();
    vec![frame; count]
}

/// Levels reported while pushing `frames`, in order.
fn levels(meter: &mut LevelMeter, frames: Vec<Vec<f32>>) -> Vec<f32> {
    frames.iter().filter_map(|f| meter.push(f)).collect()
}

fn last_level(meter: &mut LevelMeter, frames: Vec<Vec<f32>>) -> f32 {
    *levels(meter, frames).last().expect("at least one level")
}

fn assert_near(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.01, "{actual} != {expected}");
}

#[test]
fn reports_a_level_every_150ms_of_audio() {
    let mut meter = LevelMeter::new();

    // 3 s of audio = 20 windows of 150 ms.
    assert_eq!(levels(&mut meter, frames_at(-30.0, 75)).len(), 20);
}

#[test]
fn silence_reads_zero() {
    let mut meter = LevelMeter::new();

    assert_eq!(
        last_level(&mut meter, vec![vec![0.0; FRAME_SAMPLES]; 10]),
        0.0
    );
}

#[test]
fn level_spans_20_db_above_the_quietest_level_heard() {
    let mut meter = LevelMeter::new();
    levels(&mut meter, frames_at(-45.0, 10));

    assert_near(last_level(&mut meter, frames_at(-35.0, 10)), 0.5);
    assert_near(last_level(&mut meter, frames_at(-25.0, 10)), 1.0);
    assert_near(last_level(&mut meter, frames_at(-5.0, 10)), 1.0);
    assert_near(last_level(&mut meter, frames_at(-45.0, 10)), 0.0);
}

#[test]
fn the_floor_never_drops_below_minus_60_db() {
    let mut meter = LevelMeter::new();
    levels(&mut meter, frames_at(-90.0, 10));

    assert_near(last_level(&mut meter, frames_at(-50.0, 10)), 0.5);
}

#[test]
fn reset_forgets_the_floor() {
    let mut meter = LevelMeter::new();
    levels(&mut meter, frames_at(-60.0, 10));

    meter.reset();
    levels(&mut meter, frames_at(-30.0, 10));

    assert_near(last_level(&mut meter, frames_at(-20.0, 10)), 0.5);
}
