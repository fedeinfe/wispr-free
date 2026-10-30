//! The on-disk meeting file: stereo Opus/Ogg, L = You, R = Others.

use std::f32::consts::TAU;
use std::path::{Path, PathBuf};

use wf_core::audio::{FRAME_SAMPLES, SAMPLE_RATE_HZ};
use wf_meetings::{MeetingWriter, read_meeting};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wf-meetings-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Consecutive 40 ms frames of a sine, continuing from sample `start`.
fn tone(hz: f32, start: usize, samples: usize) -> Vec<f32> {
    (start..start + samples)
        .map(|n| 0.3 * (TAU * hz * n as f32 / SAMPLE_RATE_HZ as f32).sin())
        .collect()
}

fn write_tones(path: &Path, frames: usize) {
    let mut writer = MeetingWriter::create(path).unwrap();
    for n in 0..frames {
        let start = n * FRAME_SAMPLES;
        writer
            .write_frame(
                &tone(440.0, start, FRAME_SAMPLES),
                &tone(1000.0, start, FRAME_SAMPLES),
            )
            .unwrap();
    }
    writer.finish().unwrap();
}

/// Power of `signal` at `hz`, normalised so a sine of amplitude A gives about A².
fn power_at(signal: &[f32], hz: f32) -> f32 {
    let (mut re, mut im) = (0.0, 0.0);
    for (n, s) in signal.iter().enumerate() {
        let phase = TAU * hz * n as f32 / SAMPLE_RATE_HZ as f32;
        re += s * phase.cos();
        im += s * phase.sin();
    }
    4.0 * (re * re + im * im) / (signal.len() * signal.len()) as f32
}

fn rms(signal: &[f32]) -> f32 {
    (signal.iter().map(|s| s * s).sum::<f32>() / signal.len() as f32).sqrt()
}

#[test]
fn a_finished_meeting_decodes_to_the_same_duration_on_both_channels() {
    let dir = temp_dir("duration");
    let path = dir.join("meeting.opus");
    write_tones(&path, 250);

    let audio = read_meeting(&path).unwrap();

    assert_eq!(audio.you.samples().len(), 250 * FRAME_SAMPLES);
    assert_eq!(audio.others.samples().len(), 250 * FRAME_SAMPLES);
}

#[test]
fn each_channel_keeps_its_own_signal() {
    let dir = temp_dir("channels");
    let path = dir.join("meeting.opus");
    write_tones(&path, 250);

    let audio = read_meeting(&path).unwrap();

    // Skip the codec's start-up transient.
    let you = &audio.you.samples()[8_000..];
    let others = &audio.others.samples()[8_000..];
    assert!(power_at(you, 440.0) > 0.05, "You lost its tone");
    assert!(power_at(others, 1000.0) > 0.05, "Others lost its tone");
    assert!(power_at(you, 1000.0) < 1e-4, "Others leaked into You");
    assert!(power_at(others, 440.0) < 1e-4, "You leaked into Others");
}

#[test]
fn silence_on_one_channel_stays_silent_while_the_other_is_loud() {
    let dir = temp_dir("isolation");
    let path = dir.join("meeting.opus");
    let mut writer = MeetingWriter::create(&path).unwrap();
    let silence = vec![0.0; FRAME_SAMPLES];
    for n in 0..125 {
        writer
            .write_frame(&tone(440.0, n * FRAME_SAMPLES, FRAME_SAMPLES), &silence)
            .unwrap();
    }
    writer.finish().unwrap();

    let audio = read_meeting(&path).unwrap();

    assert!(rms(audio.others.samples()) < 1e-4);
}

#[test]
fn samples_stay_at_their_original_position_in_time() {
    let dir = temp_dir("timing");
    let path = dir.join("meeting.opus");
    let mut writer = MeetingWriter::create(&path).unwrap();
    let silence = vec![0.0; FRAME_SAMPLES];
    let click_frame = 50;
    for n in 0..100 {
        let mut you = silence.clone();
        if n == click_frame {
            you[..160].fill(0.8); // 10 ms click at exactly 2 s
        }
        writer.write_frame(&you, &silence).unwrap();
    }
    writer.finish().unwrap();

    let audio = read_meeting(&path).unwrap();

    let onset = audio
        .you
        .samples()
        .iter()
        .position(|s| s.abs() > 0.4)
        .unwrap();
    let expected = click_frame * FRAME_SAMPLES;
    assert!(
        onset.abs_diff(expected) <= 16,
        "click at sample {onset}, expected {expected} (within 1 ms)"
    );
}

#[test]
fn an_existing_recording_is_never_overwritten() {
    let dir = temp_dir("overwrite");
    let path = dir.join("meeting.opus");
    write_tones(&path, 25);
    let before = std::fs::read(&path).unwrap();

    assert!(MeetingWriter::create(&path).is_err());

    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn a_meeting_with_no_audio_is_a_valid_empty_file() {
    let dir = temp_dir("empty");
    let path = dir.join("meeting.opus");
    MeetingWriter::create(&path).unwrap().finish().unwrap();

    let audio = read_meeting(&path).unwrap();

    assert!(audio.you.samples().is_empty());
    assert!(audio.others.samples().is_empty());
}

#[test]
fn a_file_that_is_not_a_meeting_is_rejected() {
    let dir = temp_dir("garbage");
    let path = dir.join("meeting.opus");
    std::fs::write(&path, b"definitely not an ogg stream").unwrap();

    assert!(read_meeting(&path).is_err());
}

/// A second, unrelated Ogg implementation has to accept the framing and the checksums.
#[test]
fn the_ogg_framing_is_readable_by_an_independent_parser() {
    let dir = temp_dir("framing");
    let path = dir.join("meeting.opus");
    write_tones(&path, 100);

    let file = std::fs::File::open(&path).unwrap();
    let mut packets = ogg::PacketReader::new(std::io::BufReader::new(file));
    let head = packets.read_packet_expected().unwrap();
    let tags = packets.read_packet_expected().unwrap();
    assert_eq!(&head.data[..8], b"OpusHead");
    assert_eq!(head.data[9], 2, "two output channels");
    assert_eq!(head.data[18], 1, "RFC 7845 channel mapping family 1");
    assert_eq!(&tags.data[..8], b"OpusTags");

    let mut last = None;
    while let Some(packet) = packets.read_packet().unwrap() {
        last = Some(packet);
    }
    assert!(last.unwrap().last_in_stream(), "end of stream is marked");
}

/// Needs ffmpeg on PATH: `cargo test -p wf-meetings --test meeting_file -- --ignored`.
#[test]
#[ignore = "needs ffmpeg"]
fn ffmpeg_decodes_the_file_as_stereo_with_the_right_duration() {
    let dir = temp_dir("ffmpeg");
    let path = dir.join("meeting.opus");
    write_tones(&path, 250);

    let output = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&path)
        .args(["-f", "s16le", "-ar", "16000", "-ac", "2", "-"])
        .output()
        .unwrap();
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let samples: Vec<i16> = output
        .stdout
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    let channel = |c: usize| -> Vec<f32> {
        samples
            .iter()
            .skip(c)
            .step_by(2)
            .map(|&s| f32::from(s) / 32768.0)
            .collect()
    };
    let (left, right) = (channel(0), channel(1));
    assert!(left.len().abs_diff(250 * FRAME_SAMPLES) <= 160, "10 s long");
    assert!(power_at(&left[8_000..], 440.0) > 0.05, "L is You");
    assert!(power_at(&right[8_000..], 1000.0) > 0.05, "R is Others");
    assert!(power_at(&left[8_000..], 1000.0) < 1e-4);
}
