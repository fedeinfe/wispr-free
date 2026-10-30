//! Lets every pill state be reached by hand until the dictation pipeline drives it: the tray's
//! "Debug pill" menu (always in debug builds, with `--debug-pill` in release), `--pill-state`,
//! and a stand-in for the dictation flow behind the pill's own buttons.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::{Timer, TimerMode};
use wf_core::pill::{PillState, Sound};

use crate::mic_levels::MicLevels;
use crate::pill::{PillController, PillEvent};
use crate::{PillLook, Tray};

const LEVEL_POLL: Duration = Duration::from_millis(150);
const PROCESSING_TIME: Duration = Duration::from_millis(1500);

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

/// Connects the tray's debug menu and starts feeding the waveform a made-up level, or the real
/// one once "Use real mic level" is chosen.
pub fn install(tray: &Tray, pill: &PillController) {
    tray.set_debug_menu(true);

    let controller = pill.clone();
    tray.on_debug_state(move |look| controller.set_state(state_for(look)));

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

/// Plays the part of the dictation pipeline, which does not exist yet.
pub fn handle_event(pill: &PillController, event: PillEvent) {
    match event {
        PillEvent::ToggleHandsFree => pill.set_state(PillState::ListeningHandsFree),
        PillEvent::Cancel => pill.set_state(PillState::Resting),
        PillEvent::Stop => {
            pill.set_state(PillState::Processing);
            let pill = pill.clone();
            Timer::single_shot(PROCESSING_TIME, move || {
                if pill.state() == PillState::Processing {
                    pill.set_state(PillState::Resting);
                }
            });
        }
        PillEvent::Quit => {
            let _ = slint::quit_event_loop();
        }
        PillEvent::PasteLast
        | PillEvent::PasteRecent(_)
        | PillEvent::SelectMicrophone(_)
        | PillEvent::SelectLanguage(_)
        | PillEvent::SelectPipelineQuality(_)
        | PillEvent::OpenSettings => eprintln!("pill menu: {event:?} is not wired up yet"),
    }
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
