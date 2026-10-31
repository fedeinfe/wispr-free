//! The real [`Machine`], and what stands in front of it on a run that isn't the real app's.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use wf_core::dictation::PillInput;
use wf_dictation::Dictation;
use wf_platform::desktop;
use wf_platform::permissions::{self, Access, Permission, Permissions};

use super::Machine;
use crate::flow::KeyWatch;

pub struct SystemMachine {
    pub bundle: PathBuf,
    pub dictation: Dictation,
    pub keys: KeyWatch,
    /// Tells the tray whether to offer to finish the setup.
    pub set_setup_needed: Box<dyn Fn(bool)>,
    pub close_settings: Box<dyn Fn()>,
    /// Ends the run loop.
    pub quit: Box<dyn Fn()>,
}

impl Machine for SystemMachine {
    fn permissions(&self) -> Permissions {
        permissions::check()
    }

    fn ask(&self, permission: Permission) {
        permissions::ask(permission);
    }

    fn keys_listening(&self) -> bool {
        self.keys.is_listening()
    }

    fn refresh_keys(&self, permissions: &Permissions, may_prompt: bool) {
        self.keys.refresh(permissions, may_prompt);
    }

    fn arm_trial(&self, armed: bool) {
        self.dictation.arm_trial(armed);
    }

    fn start_trial(&self) {
        self.dictation.start_trial();
    }

    fn stop_trial(&self) {
        self.dictation.on_pill(PillInput::Stop);
    }

    fn set_blocked(&self, reason: Option<String>) {
        self.dictation.set_blocked(reason);
    }

    fn set_setup_needed(&self, needed: bool) {
        (self.set_setup_needed)(needed);
    }

    fn close_settings(&self) {
        (self.close_settings)();
    }

    fn reopen_app(&self) {
        if let Err(error) = desktop::reopen_after_quit(&self.bundle) {
            eprintln!("could not reopen the app: {error}");
            return;
        }
        (self.quit)();
    }
}

/// What a rehearsal starts from, which is how a first run reads on a Mac that has never been asked.
const NOTHING_ASKED: Permissions = Permissions {
    microphone: Access::Unknown,
    accessibility: Access::Denied,
    input_monitoring: Access::Unknown,
};

/// The machine as a dev run may touch it. The OS's questions, the key listener, the microphone and a
/// restart are for a run a person started: any other could raise a prompt nobody asked for, or race
/// the real app. A rehearsal has nothing real under it, so its permissions are made up and an "ask"
/// is answered at once.
pub struct DevMachine {
    real: Rc<dyn Machine>,
    by_hand: Arc<AtomicBool>,
    rehearsing: Arc<AtomicBool>,
    rehearsed: Cell<Permissions>,
}

impl DevMachine {
    pub fn new(
        real: Rc<dyn Machine>,
        by_hand: Arc<AtomicBool>,
        rehearsing: Arc<AtomicBool>,
    ) -> Self {
        Self {
            real,
            by_hand,
            rehearsing,
            rehearsed: Cell::new(NOTHING_ASKED),
        }
    }

    fn rehearsing(&self) -> bool {
        self.rehearsing.load(Ordering::SeqCst)
    }

    fn may_touch_the_mac(&self) -> bool {
        self.by_hand.load(Ordering::SeqCst) && !self.rehearsing()
    }
}

impl Machine for DevMachine {
    fn permissions(&self) -> Permissions {
        if self.rehearsing() {
            self.rehearsed.get()
        } else {
            self.real.permissions()
        }
    }

    fn ask(&self, permission: Permission) {
        if self.may_touch_the_mac() {
            self.real.ask(permission);
            return;
        }
        eprintln!("would ask for {permission:?}");
        if self.rehearsing() {
            let mut rehearsed = self.rehearsed.get();
            match permission {
                Permission::Microphone => rehearsed.microphone = Access::Granted,
                Permission::Accessibility => rehearsed.accessibility = Access::Granted,
                Permission::InputMonitoring => rehearsed.input_monitoring = Access::Granted,
            }
            self.rehearsed.set(rehearsed);
        }
    }

    fn keys_listening(&self) -> bool {
        self.rehearsing() || self.real.keys_listening()
    }

    fn refresh_keys(&self, permissions: &Permissions, may_prompt: bool) {
        if self.may_touch_the_mac() {
            self.real.refresh_keys(permissions, may_prompt);
        }
    }

    fn arm_trial(&self, armed: bool) {
        self.real.arm_trial(armed);
    }

    /// A rehearsal's flow plays a tone, so only here a tool may start one.
    fn start_trial(&self) {
        if self.may_touch_the_mac() || self.rehearsing() {
            self.real.start_trial();
        } else {
            eprintln!("would start a trial");
        }
    }

    fn stop_trial(&self) {
        self.real.stop_trial();
    }

    /// A run that takes no keys always has "no keys", and that's nothing to tell the pill.
    fn set_blocked(&self, reason: Option<String>) {
        if self.may_touch_the_mac() || self.rehearsing() {
            self.real.set_blocked(reason);
        }
    }

    fn set_setup_needed(&self, needed: bool) {
        self.real.set_setup_needed(needed);
    }

    fn close_settings(&self) {
        self.real.close_settings();
    }

    fn reopen_app(&self) {
        if self.may_touch_the_mac() {
            self.real.reopen_app();
        } else {
            eprintln!("would reopen the app");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// Records whatever gets through.
    #[derive(Default)]
    struct Recording(RefCell<Vec<&'static str>>);

    impl Machine for Rc<Recording> {
        fn permissions(&self) -> Permissions {
            self.0.borrow_mut().push("permissions");
            Permissions {
                microphone: Access::Granted,
                accessibility: Access::Granted,
                input_monitoring: Access::Granted,
            }
        }

        fn ask(&self, _permission: Permission) {
            self.0.borrow_mut().push("ask");
        }

        fn keys_listening(&self) -> bool {
            false
        }

        fn refresh_keys(&self, _permissions: &Permissions, _may_prompt: bool) {
            self.0.borrow_mut().push("keys");
        }

        fn arm_trial(&self, _armed: bool) {}

        fn start_trial(&self) {
            self.0.borrow_mut().push("trial");
        }

        fn stop_trial(&self) {}

        fn set_blocked(&self, _reason: Option<String>) {
            self.0.borrow_mut().push("pill");
        }

        fn set_setup_needed(&self, _needed: bool) {}

        fn close_settings(&self) {}

        fn reopen_app(&self) {
            self.0.borrow_mut().push("reopen");
        }
    }

    fn machine(by_hand: bool, rehearsing: bool) -> (DevMachine, Rc<Recording>) {
        let real = Rc::new(Recording::default());
        let dev = DevMachine::new(
            Rc::new(Rc::clone(&real)),
            Arc::new(AtomicBool::new(by_hand)),
            Arc::new(AtomicBool::new(rehearsing)),
        );
        (dev, real)
    }

    /// Everything that could raise a prompt, take the keys or the mic, or end the run.
    fn press_everything(machine: &DevMachine) {
        let permissions = machine.permissions();
        machine.ask(Permission::Microphone);
        machine.ask(Permission::InputMonitoring);
        machine.ask(Permission::Accessibility);
        machine.refresh_keys(&permissions, true);
        machine.start_trial();
        machine.set_blocked(Some("why".to_owned()));
        machine.reopen_app();
    }

    #[test]
    fn a_run_nobody_started_only_reads() {
        let (dev, real) = machine(false, false);

        press_everything(&dev);

        assert_eq!(*real.0.borrow(), ["permissions"]);
    }

    #[test]
    fn a_run_a_person_started_reaches_the_mac() {
        let (dev, real) = machine(true, false);

        press_everything(&dev);

        let reached = ["ask", "ask", "ask", "keys", "trial", "pill", "reopen"];
        assert_eq!(real.0.borrow()[1..], reached);
    }

    #[test]
    fn a_rehearsal_touches_nothing_even_by_hand_but_its_own_trial() {
        for by_hand in [false, true] {
            let (dev, real) = machine(by_hand, true);

            press_everything(&dev);

            assert_eq!(*real.0.borrow(), ["trial", "pill"]);
        }
    }

    #[test]
    fn a_rehearsal_starts_with_nothing_asked_and_answers_at_once() {
        let (dev, _) = machine(false, true);
        assert_eq!(dev.permissions(), NOTHING_ASKED);
        assert!(dev.keys_listening());

        dev.ask(Permission::Microphone);
        assert_eq!(dev.permissions().microphone, Access::Granted);
        assert_eq!(dev.permissions().input_monitoring, Access::Unknown);

        dev.ask(Permission::InputMonitoring);
        dev.ask(Permission::Accessibility);
        assert_eq!(dev.permissions().accessibility, Access::Granted);
        assert_eq!(dev.permissions().input_monitoring, Access::Granted);
    }

    #[test]
    fn a_run_that_does_not_ask_keeps_saying_what_the_real_mac_says() {
        let (dev, _) = machine(false, false);

        dev.ask(Permission::Microphone);

        assert!(!dev.keys_listening());
        assert_eq!(dev.permissions().microphone, Access::Granted);
    }
}
