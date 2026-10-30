//! Times what opening the mic on key-down costs: from starting capture to the first frame, which
//! is the speech a user loses by talking the moment they press. Repeats it to show the spread.
//!
//! ```sh
//! cargo run -p wf-audio --release --example first_frame            # 10 runs, system default mic
//! cargo run -p wf-audio --release --example first_frame -- 3 <id>  # 3 runs, ranked device ids
//! ```

use std::time::{Duration, Instant};

use wf_audio::{Capture, CaptureConfig, CaptureEvent};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let runs = args.first().map_or(Ok(10), |runs| runs.parse())?;
    let ranking = args.iter().skip(1).cloned().collect::<Vec<_>>();

    for _ in 0..runs {
        let started = Instant::now();
        let capture = Capture::start(CaptureConfig {
            ranking: ranking.clone(),
            ..CaptureConfig::default()
        })?;
        let mut device = None;
        let peak = loop {
            match capture.events().recv_timeout(Duration::from_secs(5))? {
                CaptureEvent::Device(chosen) => device = chosen,
                CaptureEvent::Frame(frame) => {
                    break frame
                        .iter()
                        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
                }
            }
        };
        let name = device.map_or_else(|| "?".to_owned(), |device| device.name);
        println!(
            "{:?} to the first frame on {name} (peak {peak:.4})",
            started.elapsed()
        );
        drop(capture);
        std::thread::sleep(Duration::from_millis(300));
    }
    Ok(())
}
