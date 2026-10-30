//! The dictation state machine: every timing rule of spec §2. Pure: it is fed key events and the
//! current time, and answers with commands for the rest of the app. Time is any monotonic
//! `Duration` from an origin the caller picks.

use std::time::Duration;

use crate::shortcut::{Action, Bindings, Key, KeyEvent};

/// The one window behind hold vs tap, double-tap and triple-tap.
pub const TAP_WINDOW: Duration = Duration::from_millis(500);
/// How long into push-to-talk an unrelated key press discards the dictation.
pub const STRAY_KEY_WINDOW: Duration = Duration::from_secs(1);
pub const CAP_WARNING: Duration = Duration::from_secs(19 * 60);
pub const CAP: Duration = Duration::from_secs(20 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    PushToTalk,
    HandsFree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReason {
    User,
    /// A push-to-talk tap shorter than the tap window, not followed by a second tap.
    TooShort,
    /// A third quick tap right after the double-tap that locked.
    TripleTap,
    /// Another key was pressed early in push-to-talk: the user was most likely typing a shortcut.
    /// Worth a hint.
    StrayKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Begin capturing (keep the pre-roll).
    Start(Mode),
    /// Push-to-talk became hands-free; recording continues.
    Lock,
    StopAndInsert,
    /// Abort the dictation and discard its audio.
    Cancel(CancelReason),
    /// One minute left before the cap.
    CapWarning,
    /// Fired once every key of the combo is up, so held modifiers can't alter the paste keystroke.
    PasteLast,
    CopyLast,
    ToggleMeetingNotes,
}

/// Clicks on the pill (spec §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PillInput {
    /// Toggles hands-free dictation.
    Click,
    Stop,
    Cancel,
}

/// What the machine made of one key event.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Reaction {
    pub commands: Vec<Command>,
    /// Keep the event from the focused app: it was a non-modifier key of a shortcut (e.g. the Space
    /// of Fn+Space, or Esc cancelling). Modifiers always pass so the OS's modifier state stays sane.
    pub swallow: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    Dictating {
        started: Duration,
        cap_warned: bool,
        phase: Phase,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Push-to-talk is held (since the dictation started).
    Holding,
    /// A short tap ended at `released`; a second press inside the tap window locks.
    TapPending { released: Duration },
    /// Hands-free. After a double-tap, `second_tap` tracks the second press so a third quick tap
    /// can cancel.
    Locked { second_tap: SecondTap },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SecondTap {
    None,
    Held,
    Released(Duration),
}

pub struct DictationMachine {
    bindings: Bindings,
    state: State,
    /// Keys currently down, in press order.
    held: Vec<Key>,
    /// Set when a press ended a dictation: shortcuts are ignored until every key is up, so the
    /// rest of that combo can't start a new one.
    latched: bool,
    /// The paste-last combo was pressed; it fires when every key is up.
    paste_armed: bool,
    /// Keys whose press was swallowed, so their release is swallowed too.
    swallowed: Vec<Key>,
    /// An earlier dictation is still being transcribed or inserted, so Cancel stays live.
    processing: bool,
}

impl DictationMachine {
    pub fn new(bindings: Bindings) -> Self {
        Self {
            bindings,
            state: State::Idle,
            held: Vec::new(),
            latched: false,
            paste_armed: false,
            swallowed: Vec::new(),
            processing: false,
        }
    }

    /// New shortcuts, for the next press. A dictation in progress carries on.
    pub fn set_bindings(&mut self, bindings: Bindings) {
        self.bindings = bindings;
    }

    /// Whether a finished recording is still on its way to insertion. The machine only knows about
    /// capture, so the app says so: Cancel then stops that work too. A new dictation may start.
    pub fn set_processing(&mut self, processing: bool) {
        self.processing = processing;
    }

    /// Forgets the running dictation, with no command: the app gave up on it by itself.
    pub fn abandon(&mut self) {
        self.state = State::Idle;
        self.latched = !self.held.is_empty();
    }

    pub fn on_key(&mut self, event: KeyEvent, now: Duration) -> Reaction {
        let mut reaction = Reaction {
            commands: self.tick(now),
            swallow: false,
        };
        let commands = &mut reaction.commands;
        if event.pressed {
            if self.held.contains(&event.key) {
                // Auto-repeat, or a second backend reporting the same key.
                reaction.swallow = self.swallowed.contains(&event.key);
                return reaction;
            }
            self.held.push(event.key);
            let consumed = if self.latched {
                self.completes_any_combo()
            } else {
                self.on_press(event.key, now, commands)
            };
            if consumed && !event.key.is_modifier() {
                self.swallowed.push(event.key);
                reaction.swallow = true;
            }
        } else {
            let Some(index) = self.held.iter().position(|&k| k == event.key) else {
                return reaction; // pressed before we started listening
            };
            self.held.remove(index);
            if let Some(index) = self.swallowed.iter().position(|&k| k == event.key) {
                self.swallowed.remove(index);
                reaction.swallow = true;
            }
            if self.held.is_empty() {
                self.latched = false;
                if std::mem::take(&mut self.paste_armed) {
                    commands.push(Command::PasteLast);
                }
            }
            if self.bindings.push_to_talk.contains(event.key)
                && !self.bindings.push_to_talk.is_held_within(&self.held)
            {
                self.on_ptt_release(now, commands);
            }
        }
        reaction
    }

    pub fn on_pill(&mut self, input: PillInput, now: Duration) -> Vec<Command> {
        let mut commands = self.tick(now);
        match (input, self.state) {
            (PillInput::Click, _) => self.on_hands_free_press(now, &mut commands),
            (_, State::Idle) => {}
            (PillInput::Stop, _) => self.end(Command::StopAndInsert, &mut commands),
            (PillInput::Cancel, _) => self.end(Command::Cancel(CancelReason::User), &mut commands),
        }
        commands
    }

    /// The backend may have missed key events (secure input, a disabled event tap). Forgets which
    /// keys are down, so a lost release can't block shortcuts forever; a held push-to-talk counts
    /// as released.
    pub fn on_keys_lost(&mut self, now: Duration) -> Vec<Command> {
        let mut commands = self.tick(now);
        self.held.clear();
        self.swallowed.clear();
        self.latched = false;
        self.paste_armed = false;
        self.on_ptt_release(now, &mut commands);
        commands
    }

    /// Applies every deadline that has passed by `now`. Call it at [`Self::next_deadline`].
    pub fn tick(&mut self, now: Duration) -> Vec<Command> {
        let mut commands = Vec::new();
        let State::Dictating {
            started,
            cap_warned,
            phase,
        } = &mut self.state
        else {
            return commands;
        };
        if let Phase::TapPending { released } = *phase
            && now >= released + TAP_WINDOW
        {
            self.state = State::Idle;
            commands.push(Command::Cancel(CancelReason::TooShort));
            return commands;
        }
        if !*cap_warned && now >= *started + CAP_WARNING {
            *cap_warned = true;
            commands.push(Command::CapWarning);
        }
        if now >= *started + CAP {
            self.state = State::Idle;
            commands.push(Command::StopAndInsert);
        }
        commands
    }

    /// When the app should next call [`Self::tick`], if anything is pending.
    pub fn next_deadline(&self) -> Option<Duration> {
        let State::Dictating {
            started,
            cap_warned,
            phase,
        } = self.state
        else {
            return None;
        };
        Some(match phase {
            Phase::TapPending { released } => released + TAP_WINDOW,
            _ if !cap_warned => started + CAP_WARNING,
            _ => started + CAP,
        })
    }

    /// Returns whether the press was used by a shortcut.
    fn on_press(&mut self, key: Key, now: Duration, commands: &mut Vec<Command>) -> bool {
        let bindings = &self.bindings;
        let cancellable = self.state != State::Idle || self.processing;
        // Push-to-talk keys are usually still down, so Cancel only needs to be among them.
        if cancellable
            && bindings.cancel.contains(key)
            && bindings.cancel.is_held_within(&self.held)
        {
            self.end(Command::Cancel(CancelReason::User), commands);
        } else if bindings.hands_free.is_held_exactly(&self.held) {
            self.on_hands_free_press(now, commands);
        } else if bindings.push_to_talk.is_held_exactly(&self.held) {
            self.on_ptt_press(now, commands);
        } else if bindings.paste_last.is_held_exactly(&self.held) {
            self.paste_armed = true;
        } else if bindings.copy_last.is_held_exactly(&self.held) {
            commands.push(Command::CopyLast);
        } else if bindings.meeting_notes.is_held_exactly(&self.held) {
            commands.push(Command::ToggleMeetingNotes);
        } else {
            if self.is_stray(key, now) {
                self.end(Command::Cancel(CancelReason::StrayKey), commands);
            }
            return false;
        }
        true
    }

    fn completes_any_combo(&self) -> bool {
        Action::ALL
            .iter()
            .any(|&action| self.bindings.get(action).is_held_exactly(&self.held))
    }

    fn is_stray(&self, key: Key, now: Duration) -> bool {
        match self.state {
            State::Dictating {
                started,
                phase: Phase::Holding | Phase::TapPending { .. },
                ..
            } => {
                now.saturating_sub(started) < STRAY_KEY_WINDOW
                    && !self.bindings.push_to_talk.contains(key)
                    && !self.bindings.hands_free.contains(key)
            }
            _ => false,
        }
    }

    fn on_hands_free_press(&mut self, now: Duration, commands: &mut Vec<Command>) {
        let locked = Phase::Locked {
            second_tap: SecondTap::None,
        };
        match &mut self.state {
            State::Idle => {
                self.state = State::Dictating {
                    started: now,
                    cap_warned: false,
                    phase: locked,
                };
                commands.push(Command::Start(Mode::HandsFree));
            }
            State::Dictating {
                phase: Phase::Locked { .. },
                ..
            } => self.end(Command::StopAndInsert, commands),
            State::Dictating { phase, .. } => {
                *phase = locked;
                commands.push(Command::Lock);
            }
        }
    }

    fn on_ptt_press(&mut self, now: Duration, commands: &mut Vec<Command>) {
        match &mut self.state {
            State::Idle => {
                self.state = State::Dictating {
                    started: now,
                    cap_warned: false,
                    phase: Phase::Holding,
                };
                commands.push(Command::Start(Mode::PushToTalk));
            }
            State::Dictating { phase, .. } => match *phase {
                Phase::TapPending { .. } => {
                    *phase = Phase::Locked {
                        second_tap: SecondTap::Held,
                    };
                    commands.push(Command::Lock);
                }
                Phase::Locked {
                    second_tap: SecondTap::Released(at),
                } if now < at + TAP_WINDOW => {
                    self.end(Command::Cancel(CancelReason::TripleTap), commands)
                }
                Phase::Locked { .. } => self.end(Command::StopAndInsert, commands),
                Phase::Holding => {}
            },
        }
    }

    fn on_ptt_release(&mut self, now: Duration, commands: &mut Vec<Command>) {
        let State::Dictating { started, phase, .. } = &mut self.state else {
            return;
        };
        match *phase {
            Phase::Holding if now.saturating_sub(*started) >= TAP_WINDOW => {
                self.state = State::Idle;
                commands.push(Command::StopAndInsert);
            }
            Phase::Holding => *phase = Phase::TapPending { released: now },
            Phase::Locked {
                second_tap: SecondTap::Held,
            } => {
                *phase = Phase::Locked {
                    second_tap: SecondTap::Released(now),
                }
            }
            _ => {}
        }
    }

    /// Ends the dictation from user input.
    fn end(&mut self, command: Command, commands: &mut Vec<Command>) {
        self.state = State::Idle;
        self.latched = !self.held.is_empty();
        commands.push(command);
    }
}
