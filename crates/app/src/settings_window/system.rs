//! The real [`Desktop`], and the probes a newly opened window runs in the background.

use std::any::Any;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use wf_audio::{AudioBackend, Capture, CaptureConfig, CaptureEvent, CpalBackend, LevelMeter};
use wf_models::hardware;
use wf_platform::autostart::LaunchAtLogin;
use wf_platform::desktop;
use wf_platform::keys::{self, KeySignal};

use super::rows::hardware_summary;
use super::{Desktop, Event, Mailbox};

#[derive(Debug, Default)]
pub struct SystemDesktop;

impl Desktop for SystemDesktop {
    fn bring_to_front(&self) {
        desktop::activate_app();
    }

    fn start_probes(&self, mailbox: &Mailbox) {
        start_probes(mailbox);
    }

    fn set_launch_at_login(&self, enabled: bool) -> io::Result<()> {
        LaunchAtLogin::system()?.set(enabled)
    }

    fn set_dock_icon_visible(&self, visible: bool) {
        desktop::set_dock_icon_visible(visible);
        desktop::activate_app();
    }

    fn open(&self, target: &str) -> io::Result<()> {
        desktop::open(target)
    }

    fn pick_folder(&self, start: &Path, done: Box<dyn FnOnce(PathBuf)>) {
        let dialog = rfd::AsyncFileDialog::new()
            .set_title("Choose a folder for models")
            .set_directory(start);
        // Not the blocking dialog: its nested run loop would re-enter Slint's event handler.
        let picked = slint::spawn_local(async move {
            if let Some(folder) = dialog.pick_folder().await {
                done(folder.path().to_path_buf());
            }
        });
        if let Err(error) = picked {
            eprintln!("could not open the folder picker: {error}");
        }
    }

    fn capture_keys(
        &self,
        mut on_signal: Box<dyn FnMut(KeySignal) + Send>,
    ) -> Result<Box<dyn Any>, String> {
        let listener = keys::listen(move |signal| {
            on_signal(signal);
            true
        })
        .map_err(|error| error.to_string())?;
        Ok(Box::new(listener))
    }

    fn test_microphone(
        &self,
        ranking: Vec<String>,
        on_level: Box<dyn FnMut(f32) + Send>,
    ) -> Result<Box<dyn Any>, String> {
        let capture = Capture::start(CaptureConfig {
            ranking,
            ..CaptureConfig::default()
        })
        .map_err(|error| error.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = Arc::clone(&stop);
            thread::spawn(move || meter_levels(&capture, &stop, on_level))
        };
        Ok(Box::new(MicrophoneTest {
            stop,
            thread: Some(thread),
        }))
    }
}

fn meter_levels(capture: &Capture, stop: &AtomicBool, mut on_level: Box<dyn FnMut(f32) + Send>) {
    let mut meter = LevelMeter::new();
    while !stop.load(Ordering::Relaxed) {
        if let Ok(CaptureEvent::Frame(frame)) =
            capture.events().recv_timeout(Duration::from_millis(100))
            && let Some(level) = meter.push(&frame)
        {
            on_level(level);
        }
    }
}

struct MicrophoneTest {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for MicrophoneTest {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Fills in what is slow to find out: the hardware summary and the list of microphones.
pub fn start_probes(mailbox: &Mailbox) {
    let hardware_mailbox = mailbox.clone();
    thread::spawn(move || {
        let hardware = hardware::detect();
        hardware_mailbox.post(Event::Hardware {
            summary: hardware_summary(&hardware),
            recommended: format!("{:?}", hardware.recommended_tier()),
        });
    });
    let microphones_mailbox = mailbox.clone();
    thread::spawn(move || {
        let backend = CpalBackend::new();
        microphones_mailbox.post(Event::Microphones {
            devices: backend.devices(),
            default: backend.default_device(),
        });
    });
}
