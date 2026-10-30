//! Runs the mic through the level meter on its own thread, for the pill waveform.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use wf_audio::{Capture, CaptureConfig, CaptureEvent, LevelMeter};

const POLL_STOP: Duration = Duration::from_millis(200);

pub struct MicLevels {
    level: Arc<AtomicU32>,
    running: Arc<AtomicBool>,
}

impl MicLevels {
    /// Opens the mic until the value is dropped. With no usable mic the level just stays at 0.
    pub fn start() -> Self {
        let level = Arc::new(AtomicU32::new(0));
        let running = Arc::new(AtomicBool::new(true));
        let (thread_level, thread_running) = (level.clone(), running.clone());
        let spawned = std::thread::Builder::new()
            .name("pill-mic-levels".into())
            .spawn(move || pump(&thread_level, &thread_running));
        if let Err(err) = spawned {
            eprintln!("could not start the mic level thread: {err}");
        }
        Self { level, running }
    }

    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }
}

impl Drop for MicLevels {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

fn pump(level: &AtomicU32, running: &AtomicBool) {
    let capture = match Capture::start(CaptureConfig::default()) {
        Ok(capture) => capture,
        Err(err) => {
            eprintln!("could not open the mic: {err}");
            return;
        }
    };
    let mut meter = LevelMeter::new();
    while running.load(Ordering::Relaxed) {
        if let Ok(CaptureEvent::Frame(frame)) = capture.events().recv_timeout(POLL_STOP)
            && let Some(latest) = meter.push(&frame)
        {
            level.store(latest.to_bits(), Ordering::Relaxed);
        }
    }
}
