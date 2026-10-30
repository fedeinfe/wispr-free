//! The pill's ten bars (spec §1): a bell-shaped height profile scaled by the smoothed mic level,
//! with a gentle wobble travelling along the bars.

use std::f32::consts::TAU;
use std::time::Duration;

pub const BARS: usize = 10;

/// Height of a bar at silence, as a fraction of the full height.
const MIN_HEIGHT: f32 = 0.12;
/// Keeps a faint shimmer going at silence, so the pill reads as listening.
const IDLE_LEVEL: f32 = 0.05;
/// Width of the bell, in bars.
const BELL_SIGMA: f32 = 2.2;
/// How far the wobble moves a bar, as a fraction of its height.
const WOBBLE_DEPTH: f32 = 0.16;
const WOBBLE_HZ: f32 = 1.5;
/// Phase step between neighbouring bars. Sets the direction and length of the travelling wave.
const WOBBLE_STEP: f32 = 0.8;
/// Time constant of the per-frame smoothing, in seconds.
const SMOOTHING_S: f32 = 0.07;

#[derive(Debug, Default)]
pub struct Waveform {
    target: f32,
    level: f32,
    phase: f32,
}

impl Waveform {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the level to approach, 0.0..=1.0, as produced by the level meter every ~150 ms.
    pub fn set_level(&mut self, level: f32) {
        self.target = level.clamp(0.0, 1.0);
    }

    /// Drops to silence without smoothing, e.g. when a dictation ends.
    pub fn reset(&mut self) {
        self.target = 0.0;
        self.level = 0.0;
    }

    /// Steps the animation by one frame and returns the bar heights, each 0.0..=1.0.
    pub fn advance(&mut self, dt: Duration) -> [f32; BARS] {
        let dt = dt.as_secs_f32();
        self.level += (self.target - self.level) * (1.0 - (-dt / SMOOTHING_S).exp());
        self.phase = (self.phase + TAU * WOBBLE_HZ * dt) % TAU;

        let amplitude = self.level.max(IDLE_LEVEL);
        std::array::from_fn(|i| {
            let wobble = 1.0 + WOBBLE_DEPTH * (self.phase - i as f32 * WOBBLE_STEP).sin();
            let height = MIN_HEIGHT + (1.0 - MIN_HEIGHT) * amplitude * bell(i) * wobble;
            height.clamp(0.0, 1.0)
        })
    }
}

fn bell(bar: usize) -> f32 {
    let from_centre = bar as f32 - (BARS - 1) as f32 / 2.0;
    (-0.5 * (from_centre / BELL_SIGMA).powi(2)).exp()
}
