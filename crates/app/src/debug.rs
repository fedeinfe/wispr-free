//! Lets every pill state be reached by hand: the tray's "Debug pill" menu (always in debug builds,
//! with `--debug-pill` in release) and `--pill-state`. `--probe-clip` runs the whole app on a
//! recording instead of the mic and keyboard.

use std::cell::{Cell, RefCell};
use std::io::{BufRead, stdin};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use slint::{Timer, TimerMode};
use wf_core::audio::AudioClip;
use wf_core::dictation::PillInput;
use wf_core::pill::{PillState, Sound};
use wf_dictation::probe::{ClipMicrophone, NoContext, NoMedia, TextLog, read_wav};
use wf_dictation::{Dictation, Parts};

use crate::mic_levels::MicLevels;
use crate::pill::PillController;
use crate::{PillLook, Tray};

const LEVEL_POLL: Duration = Duration::from_millis(150);
const PROBE_POLL: Duration = Duration::from_millis(5);

/// The app with a recording for a mic and a log for the focused app, so it can be measured beside
/// someone working: it opens no device, watches no keys and pastes nowhere. Each `dictate` on stdin
/// dictates the recording once and prints the text. `quit` or the end of stdin quits.
pub struct ClipRun {
    clip: AudioClip,
    microphone: Arc<ClipMicrophone>,
    log: TextLog,
}

impl ClipRun {
    /// Parses `--probe-clip=<16 kHz mono WAV>`.
    pub fn from_args(args: &[String]) -> Option<Self> {
        let path = args
            .iter()
            .find_map(|arg| arg.strip_prefix("--probe-clip="))?;
        let clip = read_wav(Path::new(path)).expect("the probe clip is readable");
        Some(Self {
            microphone: Arc::new(ClipMicrophone::new(clip.clone(), true)),
            clip,
            log: TextLog::default(),
        })
    }

    pub fn fit(&self, parts: &mut Parts) {
        let log = self.log.clone();
        parts.microphone = Arc::clone(&self.microphone) as _;
        parts.context = Arc::new(NoContext);
        parts.media = Arc::new(NoMedia);
        parts.insertion = Box::new(move || Box::new(log));
    }

    pub fn drive(self, flow: Dictation) {
        thread::spawn(move || {
            println!("ready");
            for line in stdin().lock().lines().map_while(Result::ok) {
                match line.as_str() {
                    "dictate" => println!("inserted: {}", self.dictate(&flow)),
                    "quit" => break,
                    _ => {}
                }
            }
            let _ = slint::quit_event_loop();
        });
    }

    fn dictate(&self, flow: &Dictation) -> String {
        let before = self.log.texts().len();
        self.microphone.set_clip(self.clip.clone());
        flow.on_pill(PillInput::Click);
        while !self.microphone.played() {
            thread::sleep(PROBE_POLL);
        }
        flow.on_pill(PillInput::Stop);
        loop {
            if let Some(text) = self.log.texts().get(before) {
                return text.clone();
            }
            thread::sleep(PROBE_POLL);
        }
    }
}

/// Parses `--pill-state=<name>`.
pub fn state_from_args(args: impl Iterator<Item = String>) -> Option<PillState> {
    args.filter_map(|arg| arg.strip_prefix("--pill-state=").map(state_named))
        .last()
        .flatten()
}

fn state_named(name: &str) -> Option<PillState> {
    Some(match name {
        "hidden" => PillState::Hidden,
        "resting" => PillState::Resting,
        "ready" => PillState::Ready,
        "listening" => PillState::ListeningPtt,
        "locked" => PillState::ListeningHandsFree,
        "processing" => PillState::Processing,
        "error" => PillState::Error {
            reason: "No microphone found".into(),
        },
        "meeting" => PillState::Meeting,
        _ => return None,
    })
}

fn state_for(look: PillLook) -> PillState {
    match look {
        PillLook::Hidden => PillState::Hidden,
        PillLook::Resting => PillState::Resting,
        PillLook::Ready => PillState::Ready,
        PillLook::Listening => PillState::ListeningPtt,
        PillLook::Locked => PillState::ListeningHandsFree,
        PillLook::Processing => PillState::Processing,
        PillLook::Error => PillState::Error {
            reason: "No microphone found".into(),
        },
        PillLook::Meeting => PillState::Meeting,
    }
}

/// Connects the tray's debug menu. Once a state is picked by hand the waveform gets a made-up
/// level, or the real one once "Use real mic level" is chosen.
pub fn install(tray: &Tray, pill: &PillController) {
    tray.set_debug_menu(true);

    let controller = pill.clone();
    let driving = Rc::new(Cell::new(false));
    let picked = driving.clone();
    tray.on_debug_state(move |look| {
        picked.set(true);
        controller.set_state(state_for(look));
    });

    let controller = pill.clone();
    tray.on_debug_sound(move |name| {
        let sound = match name.as_str() {
            "start" => Sound::Start,
            "stop" => Sound::Stop,
            "lock" => Sound::Lock,
            _ => Sound::Error,
        };
        controller.play_sound(sound);
    });

    let mic: Rc<RefCell<Option<MicLevels>>> = Rc::default();
    let chosen = mic.clone();
    tray.on_debug_mic_level(move || {
        chosen.borrow_mut().get_or_insert_with(MicLevels::start);
    });

    let controller = pill.clone();
    let started = Instant::now();
    let levels = Timer::default();
    levels.start(TimerMode::Repeated, LEVEL_POLL, move || {
        if !driving.get() {
            return;
        }
        let level = mic.borrow().as_ref().map_or_else(
            || made_up_level(started.elapsed().as_secs_f32()),
            MicLevels::level,
        );
        controller.set_level(level);
    });
    // Lives as long as the app.
    std::mem::forget(levels);
}

/// A speech-like rise and fall.
fn made_up_level(seconds: f32) -> f32 {
    let phrase = (seconds * 1.3).sin().abs();
    let syllables = 0.6 + 0.4 * (seconds * 7.0).sin();
    phrase * syllables
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_has_a_command_line_name() {
        let args = |name: &str| vec![format!("--pill-state={name}")].into_iter();
        for (name, expected) in [
            ("hidden", PillState::Hidden),
            ("resting", PillState::Resting),
            ("ready", PillState::Ready),
            ("listening", PillState::ListeningPtt),
            ("locked", PillState::ListeningHandsFree),
            ("processing", PillState::Processing),
            ("meeting", PillState::Meeting),
        ] {
            assert_eq!(state_from_args(args(name)), Some(expected));
        }
        assert!(matches!(
            state_from_args(args("error")),
            Some(PillState::Error { .. })
        ));
    }

    #[test]
    fn an_unknown_state_name_is_ignored() {
        let args = vec!["--pill-state=sideways".to_owned()].into_iter();
        assert_eq!(state_from_args(args), None);
        assert_eq!(state_from_args(std::iter::empty()), None);
    }

    #[test]
    fn the_made_up_level_stays_in_range() {
        for i in 0..10_000 {
            let level = made_up_level(i as f32 * 0.01);
            assert!((0.0..=1.0).contains(&level), "{level}");
        }
    }
}
