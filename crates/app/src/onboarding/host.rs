//! Keeps the one open onboarding window, and watches for what would bring it back.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use slint::{CloseRequestResponse, ComponentHandle, PlatformError, Timer, TimerMode};
use wf_dictation::Event as FlowEvent;

use super::steps::{Facts, Hardware, Models, Step, blocked_reason, steps_to_show};
use super::{OnboardingController, OnboardingParts};
use crate::memory;

/// Often enough that a grant shows soon after the switch in System Settings is flipped.
const LOOK_OPEN: Duration = Duration::from_secs(1);
/// With no window there's only a revoked permission to notice.
const LOOK_IDLE: Duration = Duration::from_secs(5);
/// See `settings_window::host`.
const WINDOW_TEARDOWN: Duration = Duration::from_millis(600);

/// Hands the flow's events to whoever wants them on the UI thread, once they exist. The flow has
/// to start before the window host that listens to it.
pub type FlowWatcher = Rc<RefCell<Option<Box<dyn Fn(&FlowEvent)>>>>;

/// How a window comes up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opening {
    /// The user asked for it or just launched the app.
    Foreground,
    /// A permission went away while the user works in another app, which keeps the focus.
    Quietly,
}

#[derive(Clone)]
pub struct OnboardingHost {
    shared: Rc<Shared>,
}

struct Shared {
    parts: OnboardingParts,
    open: RefCell<Option<OnboardingController>>,
    timer: Timer,
    models: Cell<Models>,
    /// What was broken when the user closed the window. Only something new reopens it.
    dismissed: RefCell<Vec<Step>>,
    blocked: RefCell<Option<String>>,
}

impl OnboardingHost {
    pub fn new(parts: OnboardingParts) -> Self {
        let models = parts.models();
        Self {
            shared: Rc::new(Shared {
                parts,
                open: RefCell::default(),
                timer: Timer::default(),
                models: Cell::new(models),
                dismissed: RefCell::default(),
                blocked: RefCell::default(),
            }),
        }
    }

    fn finished(&self) -> bool {
        self.shared.parts.store().get().onboarding.finished
    }

    /// What is true now, with what only a window can find out left for it to fill in.
    fn facts(&self) -> Facts {
        let machine = &self.shared.parts.machine;
        Facts {
            permissions: machine.permissions(),
            hardware: self
                .shared
                .open
                .borrow()
                .as_ref()
                .map_or(Hardware::NotLookedYet, OnboardingController::hardware),
            keys_listening: machine.keys_listening(),
            models: self.shared.models.get(),
        }
    }

    pub fn is_open(&self) -> bool {
        self.shared.open.borrow().is_some()
    }

    pub fn with_window<T>(&self, use_it: impl FnOnce(&OnboardingController) -> T) -> Option<T> {
        self.shared.open.borrow().as_ref().map(use_it)
    }

    /// Whether the tray should offer to finish the setup.
    pub fn setup_needed(&self) -> bool {
        !self.finished() || !self.facts().broken().is_empty()
    }

    /// At launch: opens what is needed, then keeps watching.
    pub fn start(&self) -> Result<(), PlatformError> {
        self.shared.models.set(self.shared.parts.models());
        if !steps_to_show(self.finished(), &self.facts()).is_empty() {
            self.open(Opening::Foreground)?;
        }
        let host = self.clone();
        self.shared
            .timer
            .start(TimerMode::Repeated, LOOK_IDLE, move || host.look());
        self.look();
        Ok(())
    }

    /// Opens the window on what needs doing. The tray comes here too, so there's always a way back
    /// to it. With nothing to do it doesn't open.
    pub fn open(&self, how: Opening) -> Result<(), PlatformError> {
        if let Some(open) = &*self.shared.open.borrow() {
            open.window().show()?;
            if how == Opening::Foreground {
                open.pages().bring_to_front();
            }
            return Ok(());
        }
        self.shared.models.set(self.shared.parts.models());
        let finished = self.finished();
        let facts = self.facts();
        let steps = steps_to_show(finished, &facts);
        if steps.is_empty() {
            return Ok(());
        }
        if steps.contains(&Step::Models) {
            self.shared.parts.machine.close_settings();
        }
        let host = self.clone();
        let on_done: Rc<dyn Fn()> = Rc::new(move || {
            // Not from inside the window's own callback: closing drops the window.
            let host = host.clone();
            Timer::single_shot(Duration::ZERO, move || host.close());
        });
        let controller = OnboardingController::new(
            self.shared.parts.clone(),
            steps,
            !finished,
            facts,
            Rc::clone(&on_done),
        )?;
        controller.window().window().on_close_requested(move || {
            on_done();
            CloseRequestResponse::HideWindow
        });
        controller.pages().start_probes();
        controller.window().show()?;
        if how == Opening::Foreground {
            controller.pages().bring_to_front();
        }
        *self.shared.open.borrow_mut() = Some(controller);
        self.shared.timer.set_interval(LOOK_OPEN);
        Ok(())
    }

    /// Drops the window and everything it holds, then returns the freed memory to the OS.
    pub fn close(&self) {
        if let Some(controller) = self.shared.open.borrow_mut().take() {
            let _ = controller.window().hide();
        }
        self.shared.timer.set_interval(LOOK_IDLE);
        *self.shared.dismissed.borrow_mut() = self.facts().broken();
        self.look();
        Timer::single_shot(WINDOW_TEARDOWN, memory::release_freed);
    }

    /// Brings an open window forward. What Settings does while this one is open.
    pub fn bring_forward(&self) -> bool {
        if !self.is_open() {
            return false;
        }
        if let Err(error) = self.open(Opening::Foreground) {
            eprintln!("could not show onboarding: {error}");
        }
        true
    }

    /// One re-check: the keyboard, the pill, the tray, and the window if it's open or now needed.
    pub fn look(&self) {
        if self.is_open() {
            self.shared.models.set(self.shared.parts.models());
        }
        let machine = &self.shared.parts.machine;
        let finished = self.finished();
        let before = self.facts();
        machine.refresh_keys(&before.permissions, finished || !before.keys_may_prompt());
        let facts = self.facts();
        if finished && !self.is_open() {
            self.reopen_if_something_broke(&facts);
        }
        if let Some(open) = &*self.shared.open.borrow() {
            open.look(facts);
        }
        self.tell_the_pill(&facts);
        machine.set_setup_needed(!finished || !facts.broken().is_empty());
    }

    /// While the window is open it explains things itself.
    fn tell_the_pill(&self, facts: &Facts) {
        let reason = (!self.is_open()).then(|| blocked_reason(facts)).flatten();
        let mut blocked = self.shared.blocked.borrow_mut();
        if *blocked != reason {
            blocked.clone_from(&reason);
            self.shared.parts.machine.set_blocked(reason);
        }
    }

    fn reopen_if_something_broke(&self, facts: &Facts) {
        let broken = facts.broken();
        let already_seen = {
            let mut dismissed = self.shared.dismissed.borrow_mut();
            dismissed.retain(|step| broken.contains(step));
            broken.iter().all(|step| dismissed.contains(step))
        };
        if !already_seen && let Err(error) = self.open(Opening::Quietly) {
            eprintln!("could not show onboarding: {error}");
        }
    }

    /// Something the running flow reported.
    pub fn on_flow_event(&self, event: &FlowEvent) {
        if let Some(open) = &*self.shared.open.borrow() {
            open.on_flow_event(event);
        }
    }
}
