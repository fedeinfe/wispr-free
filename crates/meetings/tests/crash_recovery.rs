//! What survives when the app dies mid-meeting.

use std::f32::consts::TAU;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use wf_core::audio::{FRAME_SAMPLES, SAMPLE_RATE_HZ};
use wf_meetings::{MeetingAudio, MeetingWriter, read_meeting, repair_meeting};

/// One Ogg page per second is the promise: a crash loses at most this much that was written.
const PAGE_SAMPLES: usize = SAMPLE_RATE_HZ as usize;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wf-meetings-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Speech-like and different on each channel, so a misplaced sample is easy to catch.
fn frame(n: usize, hz: f32) -> Vec<f32> {
    (0..FRAME_SAMPLES)
        .map(|i| {
            let t = (n * FRAME_SAMPLES + i) as f32 / SAMPLE_RATE_HZ as f32;
            let envelope = 0.6 + 0.4 * (TAU * 3.0 * t).sin();
            0.3 * envelope * (TAU * hz * t).sin()
        })
        .collect()
}

fn write_frames(writer: &mut MeetingWriter, frames: std::ops::Range<usize>) {
    for n in frames {
        writer
            .write_frame(&frame(n, 300.0), &frame(n, 700.0))
            .unwrap();
    }
}

fn record(path: &Path, frames: usize) {
    let mut writer = MeetingWriter::create(path).unwrap();
    write_frames(&mut writer, 0..frames);
    writer.finish().unwrap();
}

/// Byte offset just past each Ogg page, walked independently of the code under test.
fn page_ends(bytes: &[u8]) -> Vec<usize> {
    let mut ends = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        assert_eq!(&bytes[offset..offset + 4], b"OggS");
        let segments = usize::from(bytes[offset + 26]);
        let lacing = &bytes[offset + 27..offset + 27 + segments];
        offset += 27 + segments + lacing.iter().map(|&l| usize::from(l)).sum::<usize>();
        ends.push(offset);
    }
    ends
}

fn assert_same_start(cut: &MeetingAudio, whole: &MeetingAudio) {
    let len = cut.you.samples().len();
    assert_eq!(cut.you.samples(), &whole.you.samples()[..len]);
    assert_eq!(cut.others.samples(), &whole.others.samples()[..len]);
}

/// How many samples a file cut to `len` bytes must still give back: everything in the complete
/// pages before the cut. Page 0 and 1 are headers; each later page holds one second.
fn expected_samples(ends: &[usize], len: usize, lookahead: usize) -> usize {
    let complete = ends.iter().filter(|&&end| end <= len).count();
    (complete.saturating_sub(2) * PAGE_SAMPLES).saturating_sub(lookahead)
}

/// The decoded length of a whole file tells how much the codec's delay hides at the tail.
fn lookahead_of(bytes: &[u8], ends: &[usize], dir: &Path) -> usize {
    let path = dir.join("probe.opus");
    std::fs::write(&path, &bytes[..ends[3]]).unwrap();
    2 * PAGE_SAMPLES - read_meeting(&path).unwrap().you.samples().len()
}

#[test]
fn cutting_the_file_anywhere_keeps_every_complete_page() {
    let dir = temp_dir("cut");
    let path = dir.join("whole.opus");
    record(&path, 250); // 10 s
    let bytes = std::fs::read(&path).unwrap();
    let whole = read_meeting(&path).unwrap();
    let ends = page_ends(&bytes);
    assert!(ends.len() >= 11, "expected about a page per second");
    let lookahead = lookahead_of(&bytes, &ends, &dir);

    // Every offset around each page boundary, plus a spread through the rest of the file.
    let mut cuts: Vec<usize> = ends
        .iter()
        .flat_map(|&end| end.saturating_sub(30)..(end + 30).min(bytes.len()))
        .collect();
    cuts.extend((0..bytes.len()).step_by(97));

    let cut_path = dir.join("cut.opus");
    for cut in cuts {
        let start_of_audio = ends[1];
        std::fs::write(&cut_path, &bytes[..cut]).unwrap();
        match read_meeting(&cut_path) {
            Ok(audio) => {
                assert!(cut >= start_of_audio, "cut at {cut} should be unreadable");
                assert_same_start(&audio, &whole);
                let expected = expected_samples(&ends, cut, lookahead);
                let got = audio.you.samples().len();
                if cut < bytes.len() {
                    assert_eq!(got, expected, "cut at byte {cut}");
                }
            }
            Err(_) => assert!(cut < start_of_audio, "cut at {cut} lost the whole file"),
        }
    }
}

#[test]
fn a_flipped_byte_keeps_everything_before_the_damaged_page() {
    let dir = temp_dir("flip");
    let path = dir.join("whole.opus");
    record(&path, 250);
    let mut bytes = std::fs::read(&path).unwrap();
    let whole = read_meeting(&path).unwrap();
    let ends = page_ends(&bytes);

    bytes[ends[6] + 100] ^= 0x40; // inside the sixth audio page
    let damaged = dir.join("damaged.opus");
    std::fs::write(&damaged, &bytes).unwrap();

    let audio = read_meeting(&damaged).unwrap();

    assert_same_start(&audio, &whole);
    let kept = audio.you.samples().len();
    assert!(
        kept > 4 * PAGE_SAMPLES && kept <= 5 * PAGE_SAMPLES,
        "{kept}"
    );
}

#[test]
fn repair_turns_a_torn_file_into_a_finished_one() {
    let dir = temp_dir("repair");
    let path = dir.join("whole.opus");
    record(&path, 250);
    let bytes = std::fs::read(&path).unwrap();
    let whole = read_meeting(&path).unwrap();
    let ends = page_ends(&bytes);

    let torn = dir.join("torn.opus");
    let cut = ends[6] + 200;
    std::fs::write(&torn, &bytes[..cut]).unwrap();
    let before = read_meeting(&torn).unwrap();

    let duration = repair_meeting(&torn).unwrap();

    let after = read_meeting(&torn).unwrap();
    assert_eq!(after, before, "repair must not change the audio");
    assert_same_start(&after, &whole);
    let repaired = std::fs::read(&torn).unwrap();
    assert_eq!(repaired.len(), ends[6], "torn tail removed");
    assert_ne!(repaired[ends[5] + 5] & 0x04, 0, "end of stream is marked");
    let samples = after.you.samples().len();
    assert_eq!(duration.as_millis(), samples as u128 / 16);
}

#[test]
fn repairing_a_finished_file_changes_nothing() {
    let dir = temp_dir("repair-noop");
    let path = dir.join("whole.opus");
    record(&path, 100);
    let before = std::fs::read(&path).unwrap();

    repair_meeting(&path).unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn repair_twice_is_the_same_as_once() {
    let dir = temp_dir("repair-twice");
    let path = dir.join("whole.opus");
    record(&path, 100);
    let bytes = std::fs::read(&path).unwrap();
    let ends = page_ends(&bytes);
    let torn = dir.join("torn.opus");
    std::fs::write(&torn, &bytes[..ends[3] + 50]).unwrap();

    repair_meeting(&torn).unwrap();
    let once = std::fs::read(&torn).unwrap();
    repair_meeting(&torn).unwrap();

    assert_eq!(std::fs::read(&torn).unwrap(), once);
}

#[test]
fn a_crash_before_the_first_audio_page_repairs_to_an_empty_meeting() {
    let dir = temp_dir("repair-empty");
    let path = dir.join("whole.opus");
    record(&path, 100);
    let bytes = std::fs::read(&path).unwrap();
    let ends = page_ends(&bytes);
    let torn = dir.join("torn.opus");
    std::fs::write(&torn, &bytes[..ends[1] + 10]).unwrap();

    let duration = repair_meeting(&torn).unwrap();

    assert_eq!(duration, Duration::ZERO);
    assert!(read_meeting(&torn).unwrap().you.samples().is_empty());
}

#[test]
fn repair_leaves_a_file_it_does_not_understand_alone() {
    let dir = temp_dir("repair-garbage");
    let path = dir.join("meeting.opus");
    std::fs::write(&path, b"not a recording").unwrap();

    assert!(repair_meeting(&path).is_err());

    assert_eq!(std::fs::read(&path).unwrap(), b"not a recording");
}

/// Needs ffmpeg on PATH: `cargo test -p wf-meetings --test crash_recovery -- --ignored`.
#[test]
#[ignore = "needs ffmpeg"]
fn ffmpeg_reads_a_repaired_file_cleanly() {
    let dir = temp_dir("repair-ffmpeg");
    let path = dir.join("whole.opus");
    record(&path, 250);
    let bytes = std::fs::read(&path).unwrap();
    let ends = page_ends(&bytes);
    let torn = dir.join("torn.opus");
    std::fs::write(&torn, &bytes[..ends[6] + 200]).unwrap();

    repair_meeting(&torn).unwrap();

    let output = Command::new("ffmpeg")
        .args(["-v", "warning", "-i"])
        .arg(&torn)
        .args(["-f", "s16le", "-ar", "16000", "-ac", "2", "-"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success() && stderr.is_empty(), "{stderr}");
    let frames = output.stdout.len() / 4;
    assert!(frames.abs_diff(5 * PAGE_SAMPLES) <= 160, "{frames} frames");
}

const CHILD_PATH_VAR: &str = "WF_MEETINGS_CRASH_TEST_PATH";

/// Not a test of its own: `killing_the_writer_process` re-runs this binary into this function.
#[test]
#[ignore = "helper process for killing_the_writer_process"]
fn writer_process_that_gets_killed() {
    let Ok(path) = std::env::var(CHILD_PATH_VAR) else {
        return;
    };
    let mut writer = MeetingWriter::create(Path::new(&path)).unwrap();
    for n in 0.. {
        write_frames(&mut writer, n..n + 1);
        if (n + 1) % 25 == 0 {
            println!("wrote {} seconds", (n + 1) / 25);
        }
    }
}

#[test]
fn killing_the_writer_process_keeps_all_but_the_last_second() {
    let dir = temp_dir("kill");
    let path = dir.join("meeting.opus");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "writer_process_that_gets_killed",
            "--exact",
            "--ignored",
            "--nocapture",
        ])
        .env(CHILD_PATH_VAR, &path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let seconds = 12;
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let written = stdout
        .lines()
        .map_while(Result::ok)
        .find(|line| line == &format!("wrote {seconds} seconds"));
    assert!(written.is_some(), "writer process died early");
    child.kill().unwrap();
    child.wait().unwrap();

    let audio = read_meeting(&path).unwrap();
    let kept = audio.you.samples().len();
    assert!(
        kept > (seconds - 2) * PAGE_SAMPLES,
        "kept only {kept} samples of {seconds} s"
    );
    let expected: Vec<f32> = (0..kept / FRAME_SAMPLES)
        .flat_map(|n| frame(n, 300.0))
        .collect();
    let error: f32 = audio.you.samples()[8_000..kept - 8_000]
        .iter()
        .zip(&expected[8_000..])
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f32>()
        / (kept - 16_000) as f32;
    let signal: f32 = expected.iter().map(|s| s * s).sum::<f32>() / expected.len() as f32;
    assert!(
        error < signal * 0.1,
        "decoded audio is not what was written"
    );

    repair_meeting(&path).unwrap();
    assert_eq!(read_meeting(&path).unwrap(), audio);
}
