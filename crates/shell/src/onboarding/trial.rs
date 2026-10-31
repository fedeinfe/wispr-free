//! The "try a dictation" step: what a trial has proved so far. It proves three things together, so
//! when it fails it has to say which one.

use wf_core::pill::PillState;
use wf_dictation::TrialPart;

use super::steps::{Facts, Hardware};

/// Below this a level is the room, not a voice reaching the app.
const LEVEL_HEARD: f32 = 0.02;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Check {
    #[default]
    Waiting,
    Passed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Idle,
    Listening,
    Working,
}

/// What the step shows right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub phase: Phase,
    pub keys: Check,
    pub microphone: Check,
    pub models: Check,
    /// What to do next, or what went wrong.
    pub message: String,
    pub problem: bool,
    pub heard: Option<String>,
    /// The Stop button: the mouse started this one, so nothing else will end it.
    pub can_stop: bool,
    pub passed: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Trial {
    phase: Phase,
    keys: Check,
    microphone: Check,
    models: Check,
    heard: Option<String>,
    problem: Option<String>,
    by_mouse: bool,
    too_short: bool,
}

impl Trial {
    /// The mouse started this one, so it says nothing about the shortcut.
    pub fn started_by_mouse(&mut self) {
        self.by_mouse = true;
    }

    pub fn on_pill(&mut self, state: &PillState) {
        match state {
            PillState::ListeningPtt => {
                // Only a key gets the pill here: a click starts hands-free.
                self.keys = Check::Passed;
                self.by_mouse = false;
                self.begin();
            }
            PillState::ListeningHandsFree => self.begin(),
            PillState::Processing => self.phase = Phase::Working,
            _ => {
                // Cancelled or too short, if nothing reported an outcome before this.
                self.too_short = self.phase == Phase::Listening;
                self.phase = Phase::Idle;
            }
        }
    }

    fn begin(&mut self) {
        self.phase = Phase::Listening;
        self.problem = None;
        self.heard = None;
        self.too_short = false;
    }

    pub fn on_level(&mut self, level: f32) {
        if level >= LEVEL_HEARD {
            self.microphone = Check::Passed;
        }
    }

    pub fn on_heard(&mut self, text: String) {
        self.microphone = Check::Passed;
        self.models = Check::Passed;
        self.heard = Some(text);
        self.problem = None;
        if self.by_mouse && self.keys != Check::Passed {
            self.keys = Check::Failed;
        }
    }

    pub fn on_failed(&mut self, part: TrialPart, reason: String) {
        match part {
            TrialPart::Microphone => self.microphone = Check::Failed,
            TrialPart::Models => self.models = Check::Failed,
        }
        self.heard = None;
        self.problem = Some(reason);
    }

    /// Combines what the trial saw with what is true now: a shortcut that passed still fails if the
    /// app has since lost the keyboard.
    pub fn view(&self, facts: &Facts, shortcut: &str) -> View {
        let keys = if facts.keys_listening {
            self.keys
        } else {
            Check::Failed
        };
        let microphone = if facts.hardware == Hardware::NoMicrophone {
            Check::Failed
        } else {
            self.microphone
        };
        let passed = [keys, microphone, self.models] == [Check::Passed; 3];
        let (message, problem) = self.message(facts, keys, microphone, shortcut);
        View {
            phase: self.phase,
            keys,
            microphone,
            models: self.models,
            message,
            problem,
            heard: self.heard.clone(),
            can_stop: self.by_mouse && self.phase == Phase::Listening,
            passed,
        }
    }

    fn message(
        &self,
        facts: &Facts,
        keys: Check,
        microphone: Check,
        shortcut: &str,
    ) -> (String, bool) {
        let say = |text: String| (text, false);
        let fail = |text: String| (text, true);
        if facts.hardware == Hardware::NoMicrophone {
            return fail("No microphone connected. Plug one in and try again.".to_owned());
        }
        if !facts.keys_listening {
            return fail("The app can't see the keyboard. Go back a step to check it.".to_owned());
        }
        if let Some(problem) = &self.problem {
            return fail(problem.clone());
        }
        match self.phase {
            Phase::Listening if self.by_mouse => say("Listening… Press Stop when done.".to_owned()),
            Phase::Listening => say(format!("Listening… Let go of {shortcut} when done.")),
            Phase::Working => say("Working on it…".to_owned()),
            Phase::Idle if self.too_short => say(format!(
                "That was short. Keep {shortcut} held while you speak."
            )),
            Phase::Idle if keys == Check::Failed => fail(format!(
                "The microphone and models work, but {shortcut} never arrived. Try again, or \
                 change the shortcut below."
            )),
            Phase::Idle if self.heard.is_some() && keys == Check::Passed => {
                say(format!("It works. Hold {shortcut} in any app to dictate."))
            }
            Phase::Idle if microphone == Check::Failed => {
                fail("Nothing came from the microphone.".to_owned())
            }
            Phase::Idle => say(format!(
                "Hold {shortcut}, say a sentence, then let go. Nothing gets pasted."
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use wf_platform::permissions::{Access, Permissions};

    use super::super::steps::Models;
    use super::*;

    fn working() -> Facts {
        Facts {
            permissions: Permissions {
                microphone: Access::Granted,
                accessibility: Access::Granted,
                input_monitoring: Access::Granted,
            },
            hardware: Hardware::Microphone,
            keys_listening: true,
            models: Models::Ready,
        }
    }

    fn view(trial: &Trial) -> View {
        trial.view(&working(), "Fn")
    }

    /// A press of the real shortcut, up to the words it heard.
    fn hold_and_speak(trial: &mut Trial) {
        trial.on_pill(&PillState::ListeningPtt);
        trial.on_level(0.4);
        trial.on_pill(&PillState::Processing);
    }

    #[test]
    fn it_starts_by_saying_what_to_do() {
        let view = view(&Trial::default());
        assert_eq!(view.phase, Phase::Idle);
        assert!(view.message.starts_with("Hold Fn"), "{}", view.message);
        assert!(!view.problem);
        assert!(!view.passed);
    }

    #[test]
    fn the_shortcut_a_voice_and_words_pass_all_three() {
        let mut trial = Trial::default();

        hold_and_speak(&mut trial);
        assert_eq!(view(&trial).keys, Check::Passed);
        assert_eq!(view(&trial).microphone, Check::Passed);
        assert_eq!(view(&trial).models, Check::Waiting);
        assert_eq!(view(&trial).phase, Phase::Working);

        trial.on_heard("Testing one two.".to_owned());
        trial.on_pill(&PillState::Resting);

        let view = view(&trial);
        assert!(view.passed);
        assert_eq!(view.heard.as_deref(), Some("Testing one two."));
        assert!(view.message.starts_with("It works"), "{}", view.message);
    }

    #[test]
    fn a_failed_part_is_named_and_the_others_keep_their_result() {
        let mut trial = Trial::default();
        hold_and_speak(&mut trial);

        trial.on_failed(TrialPart::Models, "Parakeet isn't downloaded".to_owned());
        trial.on_pill(&PillState::Error {
            reason: "Parakeet isn't downloaded".to_owned(),
        });

        let view = view(&trial);
        assert_eq!(view.keys, Check::Passed);
        assert_eq!(view.microphone, Check::Passed);
        assert_eq!(view.models, Check::Failed);
        assert_eq!(view.message, "Parakeet isn't downloaded");
        assert!(view.problem);
        assert!(!view.passed);
    }

    #[test]
    fn a_mic_failure_blames_the_mic_not_the_models() {
        let mut trial = Trial::default();
        trial.on_pill(&PillState::ListeningPtt);

        trial.on_failed(TrialPart::Microphone, "No microphone found".to_owned());
        trial.on_pill(&PillState::Resting);

        let view = view(&trial);
        assert_eq!(view.microphone, Check::Failed);
        assert_eq!(view.models, Check::Waiting);
        assert_eq!(view.keys, Check::Passed);
    }

    #[test]
    fn the_mouse_proves_the_mic_and_models_and_points_at_the_shortcut() {
        let mut trial = Trial::default();
        trial.started_by_mouse();
        trial.on_pill(&PillState::ListeningHandsFree);
        assert!(view(&trial).can_stop);
        trial.on_level(0.4);
        trial.on_pill(&PillState::Processing);

        trial.on_heard("Hello.".to_owned());
        trial.on_pill(&PillState::Resting);

        let view = view(&trial);
        assert_eq!(view.microphone, Check::Passed);
        assert_eq!(view.models, Check::Passed);
        assert_eq!(view.keys, Check::Failed);
        assert!(view.message.contains("never arrived"), "{}", view.message);
        assert!(view.problem);
        assert!(!view.passed);
    }

    #[test]
    fn the_shortcut_can_still_pass_after_the_mouse_found_it_wanting() {
        let mut trial = Trial::default();
        trial.started_by_mouse();
        trial.on_pill(&PillState::ListeningHandsFree);
        trial.on_pill(&PillState::Processing);
        trial.on_heard("Hello.".to_owned());
        trial.on_pill(&PillState::Resting);

        hold_and_speak(&mut trial);
        trial.on_heard("Hello again.".to_owned());
        trial.on_pill(&PillState::Resting);

        assert!(view(&trial).passed);
    }

    #[test]
    fn a_click_on_the_pill_proves_nothing_about_the_shortcut() {
        let mut trial = Trial::default();
        trial.on_pill(&PillState::ListeningHandsFree);
        trial.on_level(0.4);
        trial.on_pill(&PillState::Processing);
        trial.on_heard("Hello.".to_owned());
        trial.on_pill(&PillState::Resting);

        assert_eq!(view(&trial).keys, Check::Waiting);
        assert!(!view(&trial).passed);
    }

    #[test]
    fn a_tap_too_short_to_count_gets_a_hint_and_keeps_the_shortcut_proved() {
        let mut trial = Trial::default();

        trial.on_pill(&PillState::ListeningPtt);
        trial.on_pill(&PillState::Resting);

        let view = view(&trial);
        assert_eq!(view.phase, Phase::Idle);
        assert_eq!(view.keys, Check::Passed);
        assert!(view.message.contains("short"), "{}", view.message);
        assert!(!view.problem);
    }

    #[test]
    fn a_new_attempt_clears_the_last_problem() {
        let mut trial = Trial::default();
        trial.on_pill(&PillState::ListeningPtt);
        trial.on_failed(TrialPart::Microphone, "No speech heard".to_owned());
        trial.on_pill(&PillState::Resting);

        trial.on_pill(&PillState::ListeningPtt);

        assert!(!view(&trial).problem);
        assert!(view(&trial).message.starts_with("Listening"));
    }

    #[test]
    fn a_quiet_mic_does_not_count_as_heard() {
        let mut trial = Trial::default();
        trial.on_pill(&PillState::ListeningPtt);
        trial.on_level(0.001);
        assert_eq!(view(&trial).microphone, Check::Waiting);
    }

    #[test]
    fn no_microphone_is_its_own_message() {
        let facts = Facts {
            hardware: Hardware::NoMicrophone,
            ..working()
        };
        let view = Trial::default().view(&facts, "Fn");
        assert_eq!(view.microphone, Check::Failed);
        assert!(
            view.message.starts_with("No microphone"),
            "{}",
            view.message
        );
        assert!(view.problem);
    }

    #[test]
    fn a_trial_that_passed_stops_passing_if_the_keyboard_goes() {
        let mut trial = Trial::default();
        hold_and_speak(&mut trial);
        trial.on_heard("Hello.".to_owned());
        trial.on_pill(&PillState::Resting);
        assert!(view(&trial).passed);

        let facts = Facts {
            keys_listening: false,
            ..working()
        };
        let view = trial.view(&facts, "Fn");
        assert_eq!(view.keys, Check::Failed);
        assert!(!view.passed);
        assert!(view.message.contains("keyboard"), "{}", view.message);
    }
}
