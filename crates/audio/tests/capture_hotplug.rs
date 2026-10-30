//! `Capture` driven through a fake backend, so plugging and unplugging needs no hardware.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use wf_audio::{
    AudioBackend, Capture, CaptureConfig, CaptureEvent, Error, InputDevice, StreamFormat,
    StreamSink,
};
use wf_core::audio::FRAME_SAMPLES;

const WAIT: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Rig {
    devices: Vec<InputDevice>,
    default: Option<String>,
    open: HashMap<String, StreamSink>,
}

#[derive(Clone, Default)]
struct FakeBackend(Arc<Mutex<Rig>>);

struct FakeStream {
    rig: Arc<Mutex<Rig>>,
    id: String,
}

impl Drop for FakeStream {
    fn drop(&mut self) {
        self.rig.lock().unwrap().open.remove(&self.id);
    }
}

impl AudioBackend for FakeBackend {
    type Stream = FakeStream;

    fn devices(&self) -> Vec<InputDevice> {
        self.0.lock().unwrap().devices.clone()
    }

    fn default_device(&self) -> Option<String> {
        self.0.lock().unwrap().default.clone()
    }

    fn open(&self, id: &str, sink: StreamSink) -> Result<(FakeStream, StreamFormat), Error> {
        let mut rig = self.0.lock().unwrap();
        if !rig.devices.iter().any(|d| d.id == id) {
            return Err(Error::DeviceUnavailable(id.to_owned()));
        }
        rig.open.insert(id.to_owned(), sink);
        let stream = FakeStream {
            rig: self.0.clone(),
            id: id.to_owned(),
        };
        // A typical mic: 48 kHz stereo.
        Ok((
            stream,
            StreamFormat {
                sample_rate: 48_000,
                channels: 2,
            },
        ))
    }
}

impl FakeBackend {
    fn with(ids: &[&str], default: &str) -> Self {
        let fake = Self::default();
        for id in ids {
            fake.plug(id);
        }
        fake.0.lock().unwrap().default = Some(default.to_owned());
        fake
    }

    fn plug(&self, id: &str) {
        self.0.lock().unwrap().devices.push(InputDevice {
            id: id.to_owned(),
            name: format!("{id} mic"),
        });
    }

    fn unplug(&self, id: &str) {
        let mut rig = self.0.lock().unwrap();
        rig.devices.retain(|d| d.id != id);
        if rig.default.as_deref() == Some(id) {
            rig.default = None;
        }
        if let Some(sink) = rig.open.get(id) {
            sink.device_lost();
        }
    }

    /// Removes `id` without its stream reporting anything, as some backends do.
    fn vanish(&self, id: &str) {
        self.0.lock().unwrap().devices.retain(|d| d.id != id);
    }

    /// Sends 40 ms of 48 kHz stereo at a constant `value` from `id`, if it is open.
    fn speak(&self, id: &str, value: f32) {
        if let Some(sink) = self.0.lock().unwrap().open.get(id) {
            sink.push(&vec![value; 1_920 * 2]);
        }
    }

    fn open_ids(&self) -> Vec<String> {
        self.0.lock().unwrap().open.keys().cloned().collect()
    }
}

fn start(fake: &FakeBackend, ranking: &[&str]) -> Capture {
    let config = CaptureConfig {
        ranking: ranking.iter().map(|s| s.to_string()).collect(),
        poll_interval: Duration::from_millis(10),
    };
    let fake = fake.clone();
    Capture::start_with(config, move || fake).unwrap()
}

/// The next device event, skipping frames.
fn next_device(capture: &Capture) -> Option<String> {
    loop {
        match capture.events().recv_timeout(WAIT).expect("a device event") {
            CaptureEvent::Device(device) => return device.map(|d| d.id),
            CaptureEvent::Frame(_) => {}
        }
    }
}

/// Speaks on `id` until a frame comes out, and returns it.
fn frame_from(capture: &Capture, fake: &FakeBackend, id: &str, value: f32) -> Vec<f32> {
    for _ in 0..200 {
        fake.speak(id, value);
        if let Ok(CaptureEvent::Frame(frame)) =
            capture.events().recv_timeout(Duration::from_millis(10))
        {
            return frame;
        }
    }
    panic!("no frame from {id}");
}

#[test]
fn captures_the_default_device_as_pipeline_frames() {
    let fake = FakeBackend::with(&["built-in", "usb"], "usb");
    let capture = start(&fake, &[]);

    assert_eq!(next_device(&capture).as_deref(), Some("usb"));
    let frame = frame_from(&capture, &fake, "usb", 0.25);
    assert_eq!(frame.len(), FRAME_SAMPLES);
}

#[test]
fn unplugging_the_device_in_use_falls_back_to_the_next_ranked_one() {
    let fake = FakeBackend::with(&["built-in", "usb", "headset"], "built-in");
    let capture = start(&fake, &["headset", "usb"]);
    assert_eq!(next_device(&capture).as_deref(), Some("headset"));

    fake.unplug("headset");

    assert_eq!(next_device(&capture).as_deref(), Some("usb"));
    assert_eq!(fake.open_ids(), ["usb"]);
    frame_from(&capture, &fake, "usb", 0.25);
}

#[test]
fn losing_every_device_reports_none_and_recovers_when_one_returns() {
    let fake = FakeBackend::with(&["usb"], "usb");
    let capture = start(&fake, &[]);
    assert_eq!(next_device(&capture).as_deref(), Some("usb"));

    fake.unplug("usb");
    assert_eq!(next_device(&capture), None);

    fake.plug("usb");
    assert_eq!(next_device(&capture).as_deref(), Some("usb"));
    frame_from(&capture, &fake, "usb", 0.25);
}

#[test]
fn plugging_in_a_preferred_device_switches_to_it() {
    let fake = FakeBackend::with(&["built-in"], "built-in");
    let capture = start(&fake, &["headset"]);
    assert_eq!(next_device(&capture).as_deref(), Some("built-in"));

    fake.plug("headset");

    assert_eq!(next_device(&capture).as_deref(), Some("headset"));
    assert_eq!(fake.open_ids(), ["headset"]);
}

#[test]
fn changing_the_ranking_switches_device() {
    let fake = FakeBackend::with(&["built-in", "usb"], "built-in");
    let capture = start(&fake, &[]);
    assert_eq!(next_device(&capture).as_deref(), Some("built-in"));

    capture.set_ranking(vec!["usb".to_owned()]);

    assert_eq!(next_device(&capture).as_deref(), Some("usb"));
}

#[test]
fn dropping_the_capture_closes_the_device() {
    let fake = FakeBackend::with(&["usb"], "usb");
    let capture = start(&fake, &[]);
    assert_eq!(next_device(&capture).as_deref(), Some("usb"));

    drop(capture);

    assert!(fake.open_ids().is_empty());
}

#[test]
fn a_device_that_vanishes_without_an_error_is_noticed_by_polling() {
    let fake = FakeBackend::with(&["usb"], "built-in");
    let capture = start(&fake, &[]);
    assert_eq!(next_device(&capture).as_deref(), Some("usb"));

    fake.vanish("usb");

    assert_eq!(next_device(&capture), None);
    assert!(fake.open_ids().is_empty());
}
