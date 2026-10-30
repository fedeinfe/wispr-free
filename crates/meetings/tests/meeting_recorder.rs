//! Recording a meeting end to end, with fake audio devices.

use std::f32::consts::TAU;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use wf_audio::{AudioBackend, Error as AudioError, InputDevice, StreamFormat, StreamSink};
use wf_core::audio::{FRAME_SAMPLES, SAMPLE_RATE_HZ};
use wf_meetings::{
    Channel, MeetingConfig, MeetingEvent, MeetingRecorder, TimelineFrame, read_meeting,
    repair_meeting,
};

const FORMAT: StreamFormat = StreamFormat {
    sample_rate: SAMPLE_RATE_HZ,
    channels: 1,
};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wf-recorder-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// One always-present device. The test feeds audio through the sink it was opened with.
#[derive(Clone, Default)]
struct FakeDevice {
    sink: Arc<Mutex<Option<StreamSink>>>,
    refuse_to_open: bool,
}

impl FakeDevice {
    fn refusing() -> Self {
        Self {
            refuse_to_open: true,
            ..Self::default()
        }
    }

    fn wait_until_open(&self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.sink.lock().unwrap().is_none() {
            assert!(Instant::now() < deadline, "the device was never opened");
            std::thread::yield_now();
        }
    }

    /// One 40 ms frame of a 400 Hz tone. Whole periods, so frames join up.
    fn push(&self, amplitude: f32) {
        let frame: Vec<f32> = (0..FRAME_SAMPLES)
            .map(|n| amplitude * (TAU * n as f32 / 40.0).sin())
            .collect();
        let guard = self.sink.lock().unwrap();
        let sink = guard.as_ref().expect("device is open");
        sink.push(&frame);
    }
}

/// Plays both devices at ten times real speed. Flat out, one channel could outrun the other by
/// more than the aligner tolerates just from thread scheduling.
fn play_both(mic: &FakeDevice, system: &FakeDevice, frames: usize) {
    for _ in 0..frames {
        mic.push(0.5);
        system.push(0.0);
        std::thread::sleep(Duration::from_millis(4));
    }
}

fn play_alone(device: &FakeDevice, frames: usize) {
    for _ in 0..frames {
        device.push(0.5);
        std::thread::sleep(Duration::from_millis(4));
    }
}

impl AudioBackend for FakeDevice {
    type Stream = ();

    fn devices(&self) -> Vec<InputDevice> {
        vec![InputDevice {
            id: "fake".into(),
            name: "Fake".into(),
        }]
    }

    fn default_device(&self) -> Option<String> {
        Some("fake".into())
    }

    fn open(&self, id: &str, sink: StreamSink) -> Result<((), StreamFormat), AudioError> {
        if self.refuse_to_open {
            return Err(AudioError::DeviceUnavailable(id.to_owned()));
        }
        *self.sink.lock().unwrap() = Some(sink);
        Ok(((), FORMAT))
    }
}

fn start(path: PathBuf, mic: &FakeDevice, system: &FakeDevice) -> MeetingRecorder {
    let config = MeetingConfig {
        poll_interval: Duration::from_millis(20),
        ..MeetingConfig::new(path)
    };
    let (mic, system) = (mic.clone(), system.clone());
    MeetingRecorder::start_with(config, move || mic, move || system).unwrap()
}

/// Waits until `count` audio frames of `channel` came through, skipping other events.
fn wait_for_frames(recorder: &MeetingRecorder, channel: Channel, count: usize) {
    let mut seen = 0;
    while seen < count {
        let event = recorder
            .events()
            .recv_timeout(Duration::from_secs(5))
            .expect("audio frames should arrive");
        if let MeetingEvent::Frame { channel: c, .. } = event
            && c == channel
        {
            seen += 1;
        }
    }
}

fn loudness(samples: &[f32]) -> f32 {
    samples.iter().map(|s| s.abs()).sum::<f32>() / samples.len() as f32
}

#[test]
fn the_mic_goes_left_and_system_audio_goes_right() {
    let dir = temp_dir("both");
    let path = dir.join("meeting.opus");
    let (mic, system) = (FakeDevice::default(), FakeDevice::default());
    let recorder = start(path.clone(), &mic, &system);
    mic.wait_until_open();
    system.wait_until_open();

    play_both(&mic, &system, 50);
    wait_for_frames(&recorder, Channel::You, 50);
    let duration = recorder.stop().unwrap();

    let audio = read_meeting(&path).unwrap();
    assert_eq!(audio.you.samples().len(), 50 * FRAME_SAMPLES);
    assert_eq!(duration, Duration::from_secs(2));
    assert!(loudness(&audio.you.samples()[4_000..12_000]) > 0.1);
    assert!(loudness(audio.others.samples()) < 1e-3);
}

#[test]
fn recording_carries_on_with_silence_when_system_audio_cannot_be_opened() {
    let dir = temp_dir("no-system");
    let path = dir.join("meeting.opus");
    let (mic, system) = (FakeDevice::default(), FakeDevice::refusing());
    let recorder = start(path.clone(), &mic, &system);
    mic.wait_until_open();

    play_alone(&mic, 50);
    wait_for_frames(&recorder, Channel::You, 50);
    recorder.stop().unwrap();

    let audio = read_meeting(&path).unwrap();
    assert_eq!(audio.you.samples().len(), 50 * FRAME_SAMPLES);
    assert!(audio.others.samples().iter().all(|s| s.abs() < 1e-3));
}

#[test]
fn device_changes_are_reported_per_channel() {
    let dir = temp_dir("devices");
    let path = dir.join("meeting.opus");
    let (mic, system) = (FakeDevice::default(), FakeDevice::refusing());
    let recorder = start(path, &mic, &system);

    let deadline = Instant::now() + Duration::from_secs(5);
    let (mut mic_up, mut system_down) = (false, false);
    while !(mic_up && system_down) {
        assert!(Instant::now() < deadline, "device events never arrived");
        if let Ok(MeetingEvent::Device { channel, device }) =
            recorder.events().recv_timeout(Duration::from_millis(100))
        {
            mic_up |= channel == Channel::You && device.is_some();
            system_down |= channel == Channel::Others && device.is_none();
        }
    }
    recorder.stop().unwrap();
}

#[test]
fn an_unread_event_stream_never_holds_the_recording_back() {
    let dir = temp_dir("unread");
    let path = dir.join("meeting.opus");
    let (mic, system) = (FakeDevice::default(), FakeDevice::default());
    let recorder = start(path.clone(), &mic, &system);
    mic.wait_until_open();
    system.wait_until_open();

    // Far more than the event buffer holds, and nobody is reading it.
    play_both(&mic, &system, 300);
    std::thread::sleep(Duration::from_millis(200));
    recorder.stop().unwrap();

    assert_eq!(
        read_meeting(&path).unwrap().you.samples().len(),
        300 * FRAME_SAMPLES
    );
}

#[test]
fn a_recording_is_never_started_over_an_existing_file() {
    let dir = temp_dir("exists");
    let path = dir.join("meeting.opus");
    std::fs::write(&path, b"an earlier meeting").unwrap();
    let (mic, system) = (FakeDevice::default(), FakeDevice::default());
    let config = MeetingConfig {
        poll_interval: Duration::from_millis(20),
        ..MeetingConfig::new(path.clone())
    };

    let result = MeetingRecorder::start_with(config, move || mic, move || system);

    assert!(result.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"an earlier meeting");
}

#[test]
fn audio_on_disk_during_the_meeting_survives_abandoning_the_recorder() {
    let dir = temp_dir("abandon");
    let path = dir.join("meeting.opus");
    let (mic, system) = (FakeDevice::default(), FakeDevice::default());
    let recorder = start(path.clone(), &mic, &system);
    mic.wait_until_open();
    system.wait_until_open();

    play_both(&mic, &system, 75);
    wait_for_frames(&recorder, Channel::You, 75);

    // The file is readable while recording is still going on.
    let deadline = Instant::now() + Duration::from_secs(5);
    while read_meeting(&path).unwrap().you.samples().len() < 2 * SAMPLE_RATE_HZ as usize {
        assert!(Instant::now() < deadline, "no page reached the disk");
        std::thread::sleep(Duration::from_millis(10));
    }
    std::mem::forget(recorder); // as good as a crash: nothing gets finished

    repair_meeting(&path).unwrap();
    assert!(read_meeting(&path).unwrap().you.samples().len() >= 2 * SAMPLE_RATE_HZ as usize);
}

fn start_with_timeline(
    path: PathBuf,
    mic: &FakeDevice,
    system: &FakeDevice,
    buffer: usize,
) -> (MeetingRecorder, Receiver<TimelineFrame>) {
    let (sender, timeline) = sync_channel(buffer);
    let config = MeetingConfig {
        poll_interval: Duration::from_millis(20),
        timeline: Some(sender),
        ..MeetingConfig::new(path)
    };
    let (mic, system) = (mic.clone(), system.clone());
    let recorder = MeetingRecorder::start_with(config, move || mic, move || system).unwrap();
    (recorder, timeline)
}

/// ScreenCaptureKit and WASAPI loopback deliver nothing while nothing plays. Counting the raw
/// frames of that channel would put the live transcript's timestamps behind the file.
#[test]
fn the_timeline_keeps_a_silent_source_in_step_with_the_file() {
    let dir = temp_dir("timeline-silent");
    let path = dir.join("meeting.opus");
    let (mic, system) = (FakeDevice::default(), FakeDevice::default());
    let (recorder, timeline) = start_with_timeline(path.clone(), &mic, &system, 1_024);
    mic.wait_until_open();
    system.wait_until_open();

    play_alone(&mic, 50);
    wait_for_frames(&recorder, Channel::You, 50);
    recorder.stop().unwrap();

    let frames: Vec<TimelineFrame> = timeline.iter().collect();
    let audio = read_meeting(&path).unwrap();
    assert_eq!(frames.len() * FRAME_SAMPLES, audio.you.samples().len());
    assert_eq!(frames.len() * FRAME_SAMPLES, audio.others.samples().len());
    let indices: Vec<u64> = frames.iter().map(|f| f.index).collect();
    assert_eq!(indices, (0..frames.len() as u64).collect::<Vec<_>>());
    assert!(loudness(&frames[10].frame.you) > 0.1);
    assert_eq!(loudness(&frames[10].frame.others), 0.0);
}

/// Nobody reading the timeline must never cost the recording anything, and the frame numbers
/// still say how much was missed.
#[test]
fn a_timeline_nobody_reads_drops_frames_and_never_the_recording() {
    let dir = temp_dir("timeline-unread");
    let path = dir.join("meeting.opus");
    let (mic, system) = (FakeDevice::default(), FakeDevice::default());
    let (recorder, timeline) = start_with_timeline(path.clone(), &mic, &system, 4);
    mic.wait_until_open();
    system.wait_until_open();

    play_both(&mic, &system, 75);
    wait_for_frames(&recorder, Channel::You, 75);
    recorder.stop().unwrap();

    let frames: Vec<TimelineFrame> = timeline.iter().collect();
    assert_eq!(frames.len(), 4);
    let audio = read_meeting(&path).unwrap();
    assert!(audio.you.samples().len() >= 70 * FRAME_SAMPLES);
}
