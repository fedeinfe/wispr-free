//! Mic level for the pill waveform (spec §1): RMS in dBFS over 150 ms windows, mapped through
//! an adaptive noise floor onto 0.0..=1.0. Per-frame smoothing is left to the UI.

use wf_core::audio::SAMPLE_RATE_HZ;

pub const LEVEL_WINDOW_MS: u32 = 150;
const WINDOW_SAMPLES: usize = (SAMPLE_RATE_HZ * LEVEL_WINDOW_MS / 1_000) as usize;
/// The floor follows the quietest window heard, but never below this.
const MIN_FLOOR_DB: f32 = -60.0;
/// Levels this far above the floor read as full scale.
const RANGE_DB: f32 = 20.0;

#[derive(Debug, Default)]
pub struct LevelMeter {
    sum_squares: f64,
    count: usize,
    floor_db: Option<f32>,
}

impl LevelMeter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds pipeline-format samples. Returns the level of the most recent 150 ms window
    /// completed by these samples, if any.
    pub fn push(&mut self, samples: &[f32]) -> Option<f32> {
        let mut level = None;
        for &s in samples {
            self.sum_squares += f64::from(s) * f64::from(s);
            self.count += 1;
            if self.count == WINDOW_SAMPLES {
                let mean_square = self.sum_squares / WINDOW_SAMPLES as f64;
                level = Some(self.level_of(10.0 * mean_square.log10() as f32));
                self.sum_squares = 0.0;
                self.count = 0;
            }
        }
        level
    }

    /// Forgets the floor, e.g. when a new dictation starts or the device changes.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    fn level_of(&mut self, db: f32) -> f32 {
        let quietest = self.floor_db.map_or(db, |floor| floor.min(db));
        let floor = quietest.max(MIN_FLOOR_DB);
        self.floor_db = Some(floor);
        ((db - floor) / RANGE_DB).clamp(0.0, 1.0)
    }
}
