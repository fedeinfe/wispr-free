//! The real [`Desktop`], and the probes a newly opened window runs in the background.

use std::any::Any;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use directories::UserDirs;
use wf_audio::{AudioBackend, Capture, CaptureConfig, CaptureEvent, CpalBackend, LevelMeter};
use wf_core::settings::{PipelineTier, SettingsStore, data_dir};
use wf_core::shortcut::Platform;
use wf_dictation::Dictation;
use wf_models::{Catalog, Tier, hardware};
use wf_platform::autostart::LaunchAtLogin;
use wf_platform::desktop;
use wf_platform::keys::{self, KeyListener, KeySignal};

use super::rows::hardware_summary;
use super::{Desktop, Event, Mailbox, OnChange, Paths, SettingsParts};

/// The folder picker is the toolkit's. `done` runs on the UI thread, only if one is picked.
pub type PickFolder = Rc<dyn Fn(&Path, Box<dyn FnOnce(PathBuf)>)>;

pub struct SystemDesktop {
    pub dictation: Dictation,
    pub pick_folder: PickFolder,
}

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
        (self.pick_folder)(start, done);
    }

    fn capture_keys(
        &self,
        mut on_signal: Box<dyn FnMut(KeySignal) + Send>,
    ) -> Result<Box<dyn Any>, String> {
        // Both listeners see every key, and dictation must not act on a shortcut being recorded.
        self.dictation.pause_keys(true);
        let listener = keys::listen(move |signal| {
            on_signal(signal);
            true
        });
        match listener {
            Ok(listener) => Ok(Box::new(KeyCapture {
                _listener: listener,
                dictation: self.dictation.clone(),
            })),
            Err(error) => {
                self.dictation.pause_keys(false);
                Err(error.to_string())
            }
        }
    }

    fn before_delete_all(&self) {
        self.dictation.forget_history();
    }

    fn look_for_microphones(&self, mailbox: &Mailbox) {
        look_for_microphones(mailbox);
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

struct KeyCapture {
    _listener: KeyListener,
    dictation: Dictation,
}

impl Drop for KeyCapture {
    fn drop(&mut self) {
        self.dictation.pause_keys(false);
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
            recommended: match hardware.recommended_tier() {
                Tier::Lite => PipelineTier::Lite,
                Tier::Balanced => PipelineTier::Balanced,
                Tier::High => PipelineTier::High,
            },
        });
    });
    look_for_microphones(mailbox);
}

/// One at a time: onboarding looks every second, and a slow audio system mustn't pile threads up.
fn look_for_microphones(mailbox: &Mailbox) {
    static LOOKING: AtomicBool = AtomicBool::new(false);
    if LOOKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let mailbox = mailbox.clone();
    thread::spawn(move || {
        let backend = CpalBackend::new();
        let (devices, default) = (backend.devices(), backend.default_device());
        LOOKING.store(false, Ordering::SeqCst);
        mailbox.post(Event::Microphones { devices, default });
    });
}

/// Where exports land: the Downloads folder, or as near as the machine has.
pub fn export_folder(data_dir: &Path) -> PathBuf {
    UserDirs::new()
        .and_then(|user| {
            let downloads = user.download_dir().map(PathBuf::from);
            downloads.or_else(|| Some(user.home_dir().to_path_buf()))
        })
        .unwrap_or_else(|| data_dir.to_path_buf())
}

/// What a window on the real machine is built from.
pub fn system_parts(
    store: Arc<SettingsStore>,
    on_change: OnChange,
    dictation: Dictation,
    pick_folder: PickFolder,
) -> SettingsParts {
    let data_dir = data_dir().unwrap_or_else(|| PathBuf::from("wispr-free"));
    let export_dir = export_folder(&data_dir);
    SettingsParts {
        store,
        catalog: Arc::new(Catalog::bundled()),
        desktop: Rc::new(SystemDesktop {
            dictation,
            pick_folder,
        }),
        on_change,
        paths: Paths {
            data_dir,
            export_dir,
            model_fence: None,
        },
        platform: Platform::current(),
    }
}
