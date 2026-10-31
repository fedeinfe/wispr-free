//! Which steps onboarding shows and what is wrong, worked out from plain facts, so every state of
//! every permission is a test away.

use wf_platform::permissions::{Access, Permission, Permissions};

#[uniffi::remote(Enum)]
pub enum Access {
    Granted,
    Denied,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Step {
    Microphone,
    Keys,
    Models,
    Trial,
}

impl Step {
    pub const ALL: [Step; 4] = [Step::Microphone, Step::Keys, Step::Models, Step::Trial];

    pub fn title(self) -> &'static str {
        match self {
            Step::Microphone => "Microphone",
            Step::Keys => "Keyboard and paste",
            Step::Models => "Models",
            Step::Trial => "Try a dictation",
        }
    }

    /// The one line under the title.
    pub fn sentence(self) -> &'static str {
        match self {
            Step::Microphone => "Wispr Free needs a microphone it's allowed to listen to.",
            Step::Keys => "Two permissions let Wispr Free hear your shortcut and paste your words.",
            Step::Models => "Your words stay on this Mac, so the models that read them live here.",
            Step::Trial => "One dictation shows that the whole chain works.",
        }
    }
}

/// Whether anything is plugged in. Permission to use a mic is a separate question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hardware {
    NotLookedYet,
    NoMicrophone,
    Microphone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Models {
    Ready,
    Missing,
    /// The chosen folder isn't there, as when its drive is unplugged.
    FolderGone,
}

/// What is true of the machine right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facts {
    pub permissions: Permissions,
    pub hardware: Hardware,
    pub keys_listening: bool,
    pub models: Models,
}

impl Facts {
    /// Whether the step has nothing left to do. The trial's own result decides that one.
    pub fn done(&self, step: Step) -> bool {
        match step {
            Step::Microphone => {
                self.permissions.microphone == Access::Granted
                    && self.hardware == Hardware::Microphone
            }
            Step::Keys => self.keys_allowed(),
            Step::Models => self.models == Models::Ready,
            Step::Trial => false,
        }
    }

    pub fn keys_allowed(&self) -> bool {
        self.permissions.accessibility == Access::Granted
            && self.permissions.input_monitoring == Access::Granted
    }

    /// What was set up once and no longer works. A mic that's unplugged isn't here, nor is a drive
    /// that's away: both are normal, and the pill says so when it matters.
    pub fn broken(&self) -> Vec<Step> {
        let mut broken = Vec::new();
        if self.permissions.microphone != Access::Granted {
            broken.push(Step::Microphone);
        }
        if !self.keys_allowed() {
            broken.push(Step::Keys);
        }
        if self.models == Models::Missing {
            broken.push(Step::Models);
        }
        broken
    }

    /// Whether listening to the keys now could bring up an OS question out of turn: the keyboard
    /// step asks, once the microphone step is behind the user.
    pub fn keys_may_prompt(&self) -> bool {
        self.permissions.input_monitoring == Access::Unknown
    }
}

/// The steps to walk through: all of them the first time, then only what broke.
pub fn steps_to_show(finished: bool, facts: &Facts) -> Vec<Step> {
    if finished {
        facts.broken()
    } else {
        Step::ALL.to_vec()
    }
}

/// Where to start: the first step that still has something to do, the last one if none has.
pub fn first_open(steps: &[Step], facts: &Facts) -> usize {
    steps
        .iter()
        .position(|&step| !facts.done(step))
        .unwrap_or(steps.len().saturating_sub(1))
}

/// What to put on the pill while no window is explaining it. The pill is the only thing left
/// showing once the window is closed.
pub fn blocked_reason(facts: &Facts) -> Option<String> {
    const WHERE: &str = "use Finish setup in the menu bar";
    let missing = |permission| facts.permissions.get(permission) != Access::Granted;
    let name = if missing(Permission::InputMonitoring) {
        "Input Monitoring"
    } else if missing(Permission::Accessibility) {
        "Accessibility"
    } else if missing(Permission::Microphone) {
        "the microphone"
    } else if !facts.keys_listening {
        return Some("The shortcut isn't reaching the app. Quit and reopen it".to_owned());
    } else {
        return None;
    };
    Some(format!("Allow {name}: {WHERE}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRANTED: Permissions = Permissions {
        microphone: Access::Granted,
        accessibility: Access::Granted,
        input_monitoring: Access::Granted,
    };

    const NOT_ASKED: Permissions = Permissions {
        microphone: Access::Unknown,
        accessibility: Access::Denied,
        input_monitoring: Access::Unknown,
    };

    fn all_good() -> Facts {
        Facts {
            permissions: GRANTED,
            hardware: Hardware::Microphone,
            keys_listening: true,
            models: Models::Ready,
        }
    }

    fn with(permissions: Permissions) -> Facts {
        Facts {
            permissions,
            ..all_good()
        }
    }

    #[test]
    fn a_first_run_walks_every_step_whatever_is_already_there() {
        assert_eq!(steps_to_show(false, &all_good()), Step::ALL);
        assert_eq!(steps_to_show(false, &with(NOT_ASKED)), Step::ALL);
    }

    #[test]
    fn a_finished_user_with_everything_working_sees_nothing() {
        assert!(steps_to_show(true, &all_good()).is_empty());
    }

    #[test]
    fn a_revoked_permission_brings_up_only_its_own_step() {
        let revoked = |microphone, accessibility, input_monitoring| {
            steps_to_show(
                true,
                &with(Permissions {
                    microphone,
                    accessibility,
                    input_monitoring,
                }),
            )
        };
        let (ok, no) = (Access::Granted, Access::Denied);
        assert_eq!(revoked(no, ok, ok), [Step::Microphone]);
        assert_eq!(revoked(ok, no, ok), [Step::Keys]);
        assert_eq!(revoked(ok, ok, no), [Step::Keys]);
        assert_eq!(revoked(no, no, no), [Step::Microphone, Step::Keys]);
    }

    #[test]
    fn a_permission_the_os_no_longer_knows_about_counts_as_revoked() {
        let forgotten = Permissions {
            input_monitoring: Access::Unknown,
            ..GRANTED
        };
        assert_eq!(steps_to_show(true, &with(forgotten)), [Step::Keys]);
    }

    #[test]
    fn missing_models_come_back_but_an_unplugged_drive_does_not() {
        let missing = Facts {
            models: Models::Missing,
            ..all_good()
        };
        assert_eq!(steps_to_show(true, &missing), [Step::Models]);

        let away = Facts {
            models: Models::FolderGone,
            ..all_good()
        };
        assert!(steps_to_show(true, &away).is_empty());
        assert!(!away.done(Step::Models));
    }

    #[test]
    fn an_unplugged_mic_is_not_broken_but_the_step_waits_for_one() {
        let unplugged = Facts {
            hardware: Hardware::NoMicrophone,
            ..all_good()
        };
        assert!(steps_to_show(true, &unplugged).is_empty());
        assert!(!unplugged.done(Step::Microphone));

        let plugged_in = Facts {
            hardware: Hardware::Microphone,
            ..unplugged
        };
        assert!(plugged_in.done(Step::Microphone));
    }

    #[test]
    fn the_microphone_step_waits_for_the_first_look() {
        let not_looked = Facts {
            hardware: Hardware::NotLookedYet,
            ..all_good()
        };
        assert!(!not_looked.done(Step::Microphone));
        assert_eq!(first_open(&Step::ALL, &not_looked), 0);
    }

    #[test]
    fn no_microphone_and_no_permission_are_told_apart() {
        let neither = Facts {
            permissions: NOT_ASKED,
            hardware: Hardware::NoMicrophone,
            ..all_good()
        };
        assert!(!neither.done(Step::Microphone));
        assert_eq!(neither.broken()[0], Step::Microphone);
    }

    #[test]
    fn the_flow_starts_at_the_first_unfinished_step() {
        let steps = Step::ALL;
        assert_eq!(first_open(&steps, &with(NOT_ASKED)), 0);

        let mic_only = Permissions {
            microphone: Access::Granted,
            ..NOT_ASKED
        };
        assert_eq!(first_open(&steps, &with(mic_only)), 1);

        let no_models = Facts {
            models: Models::Missing,
            ..all_good()
        };
        assert_eq!(first_open(&steps, &no_models), 2);
        assert_eq!(first_open(&steps, &all_good()), 3);
        assert_eq!(first_open(&[Step::Keys], &all_good()), 0);
    }

    #[test]
    fn the_keyboard_isnt_touched_while_doing_so_could_prompt() {
        assert!(with(NOT_ASKED).keys_may_prompt());
        assert!(!with(GRANTED).keys_may_prompt());
        let answered = Permissions {
            input_monitoring: Access::Denied,
            ..NOT_ASKED
        };
        assert!(!with(answered).keys_may_prompt());
    }

    #[test]
    fn the_pill_names_what_to_allow_and_where() {
        assert_eq!(blocked_reason(&all_good()), None);

        let reason = blocked_reason(&with(NOT_ASKED)).unwrap();
        assert!(reason.contains("Input Monitoring"), "{reason}");
        assert!(reason.contains("Finish setup"), "{reason}");

        let mic = Permissions {
            microphone: Access::Denied,
            ..GRANTED
        };
        assert!(
            blocked_reason(&with(mic))
                .unwrap()
                .contains("the microphone")
        );
    }

    #[test]
    fn allowed_keys_that_dont_arrive_ask_for_a_reopen() {
        let stuck = Facts {
            keys_listening: false,
            ..all_good()
        };
        assert!(blocked_reason(&stuck).unwrap().contains("reopen"));
    }
}
