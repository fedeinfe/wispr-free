//! Tries meeting capture by hand.
//!
//! ```sh
//! cargo run -p wf-meetings --example meeting -- record out.opus 60  # mic + system audio
//! cargo run -p wf-meetings --example meeting -- info out.opus       # what a file holds
//! cargo run -p wf-meetings --example meeting -- repair out.opus     # finish a crashed file
//! cargo run -p wf-meetings --example meeting -- clients             # who is using the mic
//! cargo run -p wf-meetings --example meeting -- tap-test            # macOS, see below
//! ```
//!
//! `record` captures whatever the machine plays, so only run it on audio you may record. Kill it
//! mid-run to try crash recovery: `info` still reads the file and `repair` finishes it.
//!
//! `tap-test` proves the macOS tap end to end without touching anything else on the machine: it
//! plays its own tone through `afplay`, taps only that process and mutes it, records that with no
//! microphone to a temporary file, reads the file back and deletes it.

use std::path::Path;
use std::time::{Duration, Instant};

use wf_meetings::{
    Channel, MeetingConfig, MeetingDetector, MeetingEvent, MeetingRecorder, classify_client,
    mic_clients, read_meeting, repair_meeting,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["record", path] => record(path, 30),
        ["record", path, seconds] => record(path, seconds.parse()?),
        ["info", path] => info(path),
        ["repair", path] => {
            let length = repair_meeting(Path::new(path))?;
            println!("finished {path}: {:.1} s", length.as_secs_f64());
            Ok(())
        }
        ["clients"] => clients(),
        #[cfg(target_os = "macos")]
        ["tap-test"] => tap_test::run(),
        _ => Err("usage: meeting record <out.opus> [seconds] | info <file> | repair <file> | clients | tap-test".into()),
    }
}

fn record(path: &str, seconds: u64) -> Result<(), Box<dyn std::error::Error>> {
    let recorder = MeetingRecorder::start(MeetingConfig::new(path.into()))?;
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let (mut you, mut others) = (Peak::default(), Peak::default());
    let mut next_report = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        match recorder.events().recv_timeout(Duration::from_millis(100)) {
            Ok(MeetingEvent::Frame { channel, samples }) => match channel {
                Channel::You => you.push(&samples),
                Channel::Others => others.push(&samples),
            },
            Ok(MeetingEvent::Device { channel, device }) => {
                let name = device.map_or("none".to_owned(), |device| device.name);
                println!("{channel:?}: {name}");
            }
            Ok(MeetingEvent::Failed(reason)) => return Err(reason.into()),
            Err(_) => {}
        }
        if Instant::now() >= next_report {
            println!("you {} | others {}", you.report(), others.report());
            next_report += Duration::from_secs(1);
        }
    }
    let length = recorder.stop()?;
    println!("wrote {:.1} s to {path}", length.as_secs_f64());
    Ok(())
}

/// Loudest sample and frame count since the last report.
#[derive(Default)]
struct Peak {
    peak: f32,
    frames: u32,
}

impl Peak {
    fn push(&mut self, samples: &[f32]) {
        self.frames += 1;
        for sample in samples {
            self.peak = self.peak.max(sample.abs());
        }
    }

    fn report(&mut self) -> String {
        let line = format!("{:>2} frames, peak {:.3}", self.frames, self.peak);
        *self = Self::default();
        line
    }
}

fn info(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let audio = read_meeting(Path::new(path))?;
    let peak = |samples: &[f32]| samples.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
    println!("{:.1} s", audio.you.duration().as_secs_f64());
    println!("you peak {:.3}", peak(audio.you.samples()));
    println!("others peak {:.3}", peak(audio.others.samples()));
    Ok(())
}

fn clients() -> Result<(), Box<dyn std::error::Error>> {
    let clients = mic_clients();
    if clients.is_empty() {
        println!("nothing is using the mic");
    }
    for client in &clients {
        let app = classify_client(&client.name);
        println!("{:>7} {} -> {app:?}", client.pid, client.name);
    }
    let mut detector = MeetingDetector::new(true);
    for _ in 0..2 {
        if let Some(app) = detector.observe(&clients, false) {
            println!("would offer: take notes for {}?", app.label());
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
mod tap_test {
    use std::f32::consts::TAU;
    use std::process::Command;
    use std::time::{Duration, Instant};

    use wf_audio::{AudioBackend, Error, InputDevice, StreamFormat, StreamSink};
    use wf_core::audio::SAMPLE_RATE_HZ;
    use wf_meetings::{
        MeetingConfig, MeetingEvent, MeetingRecorder, SystemAudioBackend, read_meeting,
    };

    const TONE_HZ: f32 = 1000.0;
    const RATE: u32 = 44_100;

    /// Keeps the real microphone out of the test.
    struct NoMic;

    impl AudioBackend for NoMic {
        type Stream = ();

        fn devices(&self) -> Vec<InputDevice> {
            Vec::new()
        }

        fn default_device(&self) -> Option<String> {
            None
        }

        fn open(&self, id: &str, _sink: StreamSink) -> Result<((), StreamFormat), Error> {
            Err(Error::DeviceUnavailable(id.to_owned()))
        }
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!("wf-tap-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let tone = dir.join("tone.wav");
        std::fs::write(&tone, tone_wav(6))?;
        let recording = dir.join("meeting.opus");

        let mut player = Command::new("afplay")
            .args(["-v", "0.1"])
            .arg(&tone)
            .spawn()?;
        // afplay gets its CoreAudio process object once it starts playing.
        std::thread::sleep(Duration::from_millis(500));
        let pid = player.id();
        let recorder = MeetingRecorder::start_with(
            MeetingConfig::new(recording.clone()),
            || NoMic,
            move || SystemAudioBackend::muted_process(pid),
        )?;

        let started = Instant::now();
        let mut first_frame = None;
        let deadline = started + Duration::from_secs(4);
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            if let Ok(MeetingEvent::Frame { .. }) = recorder.events().recv_timeout(left) {
                first_frame.get_or_insert_with(|| started.elapsed());
            }
        }
        let length = recorder.stop()?;
        let _ = player.kill();
        let _ = player.wait();
        let audio = read_meeting(&recording);
        let _ = std::fs::remove_dir_all(&dir);
        let audio = audio?;

        let samples = audio.others.samples();
        let peak = samples.iter().fold(0f32, |peak, s| peak.max(s.abs()));
        // Past the codec's start-up, up to where the tone is still playing.
        let settled = samples
            .get(8_000..samples.len().saturating_sub(8_000))
            .unwrap_or(&[]);
        let crossings = settled
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        let measured = crossings as f64 * f64::from(SAMPLE_RATE_HZ) / settled.len().max(1) as f64;
        println!(
            "first frame after {first_frame:?}, wrote {:.2} s",
            length.as_secs_f64()
        );
        println!("others: peak {peak:.3}, {measured:.0} Hz (played {TONE_HZ} Hz)");
        match first_frame {
            None => Err("the tap delivered no audio".into()),
            Some(_) if peak < 0.005 => {
                println!("the tap runs but is silent: System Audio Recording is not allowed");
                Ok(())
            }
            Some(_) => {
                println!("the tap works");
                Ok(())
            }
        }
    }

    /// 16-bit mono PCM.
    fn tone_wav(seconds: u32) -> Vec<u8> {
        let samples = RATE * seconds;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + samples * 2).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&RATE.to_le_bytes());
        wav.extend_from_slice(&(RATE * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(samples * 2).to_le_bytes());
        for n in 0..samples {
            let sample = 0.2 * (TAU * TONE_HZ * n as f32 / RATE as f32).sin();
            wav.extend_from_slice(&((sample * 32767.0) as i16).to_le_bytes());
        }
        wav
    }
}
