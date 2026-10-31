//! What the pill shows and what its card and timers do, around the pure model in
//! `wf_core::pill`. A front end feeds it one tick per poll, draws the [`PillFrame`] that comes back
//! and asks again after `next_tick_ms`.

use std::path::Path;
use std::time::Duration;

use wf_audio::InputDevice;
use wf_core::pill::dock::{Display, Point, Rect, Size};
use wf_core::pill::waveform::Waveform;
use wf_core::pill::{Pill, PillCommand, PillState, Sound, Tick, WINDOW_SIZE};

use crate::mic_levels::MicLevels;
use crate::prefs::Prefs;
use crate::settings::page::SettingsPage;

/// Poll rates: animating, watching the cursor, and hidden.
pub const FRAME_POLL_MS: u32 = 16;
pub const CURSOR_POLL_MS: u32 = 40;
pub const HIDDEN_POLL_MS: u32 = 250;
/// How often to re-read the screens and re-assert the stacking order.
const DISPLAY_POLL: Duration = Duration::from_secs(1);
const HIDE_DURATION: Duration = Duration::from_secs(60 * 60);
const TOAST_DURATION: Duration = Duration::from_secs(4);
/// How long a question stays up when nobody answers it.
const CARD_SHOWN: Duration = Duration::from_secs(30);

/// What the card is asking, kept to answer it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Question {
    TakeNotes {
        app: String,
    },
    /// A finished dictation with nowhere to go.
    CopyText(String),
}

/// The card's words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
    pub text: String,
    pub accept: &'static str,
    pub decline: &'static str,
}

impl Question {
    pub fn asked(&self) -> Asked {
        match self {
            Question::TakeNotes { app } => Asked {
                text: format!("Take notes for this {app} meeting?"),
                accept: "Take notes",
                decline: "Not now",
            },
            Question::CopyText(_) => Asked {
                text: "No text field here. Copy your text?".to_owned(),
                accept: "Copy",
                decline: "Dismiss",
            },
        }
    }
}

/// The card on screen.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PillCard {
    /// Within the window, in points.
    pub place: Rect,
    pub text: String,
    pub accept: String,
    pub decline: String,
}

/// What to do with the window after a tick.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PillFrame {
    pub shown: PillState,
    /// Capsule within the window, in points.
    pub capsule: Rect,
    pub card: Option<PillCard>,
    /// New window position, top left of the window in desktop units, when it changed.
    pub move_to: Option<Point>,
    /// Whether the window catches the mouse. Otherwise clicks pass through to what is below.
    pub hit_test: bool,
    /// The toast, or else why the pill is in error.
    pub tooltip: Option<String>,
    /// Bar heights, each 0.0..=1.0. Empty shows no bars.
    pub bars: Vec<f32>,
    /// Full height of the bars relative to the Listening capsule.
    pub bar_scale: f32,
    /// When to tick next.
    pub next_tick_ms: u32,
    /// The next tick should bring the screens.
    pub wants_displays: bool,
}

/// Where the waveform's level comes from. Only the debug menu changes it.
enum Levels {
    Flow,
    MadeUp,
    Mic(MicLevels),
}

pub struct PillCore {
    pill: Pill,
    waveform: Waveform,
    levels: Levels,
    prefs: Prefs,
    toast: Option<(String, Duration)>,
    card_until: Option<Duration>,
    question: Option<Question>,
    recent: Vec<String>,
    microphones: Vec<InputDevice>,
    displays: Vec<Display>,
    next_display_poll: Duration,
    now: Duration,
    shown: PillState,
}

impl PillCore {
    pub fn new(prefs: Prefs) -> Self {
        Self {
            pill: Pill::new(prefs.edge),
            waveform: Waveform::new(),
            levels: Levels::Flow,
            prefs,
            toast: None,
            card_until: None,
            question: None,
            recent: Vec::new(),
            microphones: Vec::new(),
            displays: Vec::new(),
            next_display_poll: Duration::ZERO,
            now: Duration::ZERO,
            shown: PillState::Resting,
        }
    }

    pub fn state(&self) -> &PillState {
        self.pill.state()
    }

    pub fn prefs(&self) -> &Prefs {
        &self.prefs
    }

    pub fn set_settings_page(&mut self, page: SettingsPage) {
        self.prefs.settings_page = page;
    }

    /// The menu is only for a pill showing Ready.
    pub fn menu_is_open_to(&self) -> bool {
        self.shown == PillState::Ready
    }

    pub fn recent(&self) -> &[String] {
        &self.recent
    }

    pub fn microphones(&self) -> &[InputDevice] {
        &self.microphones
    }

    /// Returns the cue to play, if there is one and sounds are on.
    pub fn set_state(&mut self, state: PillState) -> Option<Sound> {
        if state.bar_scale().is_none() {
            self.waveform.reset();
        }
        let cue = self.pill.set_state(state);
        cue.filter(|_| self.prefs.sounds)
    }

    /// The Sounds setting.
    pub fn set_sounds(&mut self, sounds: bool) {
        self.prefs.sounds = sounds;
    }

    /// The Show pill setting.
    pub fn set_shown(&mut self, shown: bool) {
        self.pill.set_turned_off(!shown);
    }

    /// A short message by the pill. It shows even with the pill hidden, because the text it is
    /// about went somewhere the user can't see.
    pub fn show_toast(&mut self, text: String) {
        self.toast = Some((text, self.now + TOAST_DURATION));
    }

    /// Asks whether to take notes for a meeting in `app`. The card goes away by itself.
    pub fn suggest_meeting(&mut self, app: String) {
        self.ask(Question::TakeNotes { app });
    }

    /// Asks whether to copy a dictation that had no text field to go to.
    pub fn offer_copy(&mut self, text: String) {
        self.ask(Question::CopyText(text));
    }

    fn ask(&mut self, question: Question) {
        self.question = Some(question);
        self.pill.set_card(true);
        self.card_until = Some(self.now + CARD_SHOWN);
    }

    pub fn question(&self) -> Option<&Question> {
        self.question.as_ref()
    }

    /// Takes down the meeting question, which is out of date once a meeting records. A question
    /// about the user's text stays.
    pub fn hide_meeting_card(&mut self) {
        if matches!(self.question, Some(Question::TakeNotes { .. })) {
            self.take_question();
        }
    }

    /// Takes the question down and hands back what was asked.
    pub fn take_question(&mut self) -> Option<Question> {
        self.pill.set_card(false);
        self.card_until = None;
        self.question.take()
    }

    pub fn set_recent(&mut self, transcripts: Vec<String>) {
        self.recent = transcripts;
    }

    pub fn set_microphones(&mut self, devices: Vec<InputDevice>) {
        self.microphones = devices;
    }

    /// Latest level from the meter, 0.0..=1.0.
    pub fn set_level(&mut self, level: f32) {
        self.waveform.set_level(level);
    }

    pub fn hide_for_an_hour(&mut self) {
        self.pill.hide_for(HIDE_DURATION);
    }

    /// The left button went down on the capsule.
    pub fn press(&mut self) {
        self.pill.press();
    }

    /// The left button came up. A new edge is kept in the prefs: save them.
    pub fn release(&mut self) -> Option<PillCommand> {
        let command = self.pill.release();
        if let Some(PillCommand::EdgeChanged(edge)) = command {
            self.prefs.edge = edge;
        }
        command
    }

    /// Whether the screens read last are out of date. A tick with none uses those.
    pub fn wants_displays(&self, now: Duration) -> bool {
        now >= self.next_display_poll
    }

    /// One poll of the outside world. `now` runs from any origin and never goes back.
    pub fn tick(
        &mut self,
        now: Duration,
        cursor: Option<Point>,
        displays: Option<Vec<Display>>,
        escape: bool,
    ) -> PillFrame {
        let dt = now.saturating_sub(self.now);
        self.now = now;
        if let Some(displays) = displays {
            self.displays = displays;
            self.next_display_poll = now + DISPLAY_POLL;
        }
        if self.card_until.is_some_and(|until| now >= until) {
            self.take_question();
        }
        if self.toast.as_ref().is_some_and(|(_, until)| now >= *until) {
            self.toast = None;
        }
        let level = match &self.levels {
            Levels::Flow => None,
            Levels::MadeUp => Some(made_up_level(now.as_secs_f32())),
            Levels::Mic(mic) => Some(mic.level()),
        };
        if let Some(level) = level {
            self.waveform.set_level(level);
        }

        let frame = self.pill.tick(&Tick {
            now,
            cursor,
            displays: &self.displays,
            escape,
        });
        self.shown = frame.shown.clone();
        let toast = self.toast.as_ref().map(|(text, _)| text.as_str());
        let tooltip = toast.or(frame.shown.tooltip()).map(str::to_owned);
        let card = frame
            .card
            .zip(self.question.as_ref())
            .map(|(place, question)| {
                let asked = question.asked();
                PillCard {
                    place,
                    text: asked.text,
                    accept: asked.accept.to_owned(),
                    decline: asked.decline.to_owned(),
                }
            });
        let bar_scale = frame.shown.bar_scale();
        let next_tick_ms = match frame.shown {
            PillState::Hidden if card.is_none() => HIDDEN_POLL_MS,
            _ if bar_scale.is_some() || frame.hit_test => FRAME_POLL_MS,
            _ => CURSOR_POLL_MS,
        };
        PillFrame {
            capsule: frame.capsule,
            card,
            move_to: frame.move_to,
            hit_test: frame.hit_test,
            tooltip,
            bars: bar_scale.map_or_else(Vec::new, |_| self.waveform.advance(dt).to_vec()),
            bar_scale: bar_scale.unwrap_or(1.0),
            next_tick_ms,
            wants_displays: self.wants_displays(now),
            shown: frame.shown,
        }
    }

    /// The debug menu picked a state by hand: the waveform gets a made-up level from then on.
    pub fn drive_levels_by_hand(&mut self) {
        if matches!(self.levels, Levels::Flow) {
            self.levels = Levels::MadeUp;
        }
    }

    /// The debug menu's "Use real mic level". Opens the microphone.
    pub fn use_real_mic_level(&mut self) {
        self.levels = Levels::Mic(MicLevels::start());
    }
}

/// Saves `prefs`, which only a drag or a page of Settings changes, so a failure costs no more than
/// the place.
pub fn save_prefs(prefs: &Prefs, path: &Path) {
    if let Err(err) = prefs.save(path) {
        eprintln!("could not save pill preferences: {err}");
    }
}

/// The transparent window the pill lives in, in points.
#[uniffi::export]
pub fn pill_window_size() -> Size {
    WINDOW_SIZE
}

/// A state by the name `--pill-state` takes.
#[uniffi::export]
pub fn pill_state_named(name: &str) -> Option<PillState> {
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

/// A speech-like rise and fall.
pub fn made_up_level(seconds: f32) -> f32 {
    let phrase = (seconds * 1.3).sin().abs();
    let syllables = 0.6 + 0.4 * (seconds * 7.0).sin();
    phrase * syllables
}
