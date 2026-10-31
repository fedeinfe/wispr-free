//! Keeps the one open onboarding window, and watches for what would bring it back.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use wf_dictation::Event as FlowEvent;

use super::steps::{Facts, Hardware, Models, Step, blocked_reason, steps_to_show};
use super::{Onboarding, OnboardingParts};
use crate::clock::{Clock, Timer};
use crate::memory;
use crate::windows::{WINDOW_TEARDOWN, Window};

/// Often enough that a grant shows soon after the switch in System Settings is flipped.
const LOOK_OPEN: Duration = Duration::from_secs(1);
/// With no window there's only a revoked permission to notice.
const LOOK_IDLE: Duration = Duration::from_secs(5);

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

/// What a window opens on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Walk {
    pub steps: Vec<Step>,
    pub first_run: bool,
    pub facts: Facts,
}

/// An open window, as the host sees it.
pub trait OnboardingWindow: Window {
    fn onboarding(&self) -> &Onboarding;
}

/// Builds a window, each time one opens. `on_done` is for when it has nothing left to do.
pub type Make<W> = Rc<dyn Fn(OnboardingParts, Walk, Rc<dyn Fn()>) -> Result<W, String>>;

pub struct OnboardingHost<W> {
    shared: Rc<Shared<W>>,
}

impl<W> Clone for OnboardingHost<W> {
    fn clone(&self) -> Self {
        Self {
            shared: Rc::clone(&self.shared),
        }
    }
}

struct Shared<W> {
    parts: OnboardingParts,
    make: Make<W>,
    clock: Rc<dyn Clock>,
    open: RefCell<Option<W>>,
    timer: Box<dyn Timer>,
    models: Cell<Models>,
    /// The timer is looking, which starts at launch or with the first window.
    watching: Cell<bool>,
    /// What was broken when the user closed the window. Only something new reopens it.
    dismissed: RefCell<Vec<Step>>,
    blocked: RefCell<Option<String>>,
}

impl<W: OnboardingWindow + 'static> OnboardingHost<W> {
    pub fn new(parts: OnboardingParts, make: Make<W>, clock: Rc<dyn Clock>) -> Self {
        let models = parts.models();
        let timer = clock.timer();
        Self {
            shared: Rc::new(Shared {
                parts,
                make,
                clock,
                open: RefCell::default(),
                timer,
                models: Cell::new(models),
                watching: Cell::default(),
                dismissed: RefCell::default(),
                blocked: RefCell::default(),
            }),
        }
    }

    fn weak(&self) -> Weak<Shared<W>> {
        Rc::downgrade(&self.shared)
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
                .map_or(Hardware::NotLookedYet, |open| open.onboarding().hardware()),
            keys_listening: machine.keys_listening(),
            models: self.shared.models.get(),
        }
    }

    pub fn is_open(&self) -> bool {
        self.shared.open.borrow().is_some()
    }

    pub fn with_window<T>(&self, use_it: impl FnOnce(&W) -> T) -> Option<T> {
        self.shared.open.borrow().as_ref().map(use_it)
    }

    /// Whether the tray should offer to finish the setup.
    pub fn setup_needed(&self) -> bool {
        !self.finished() || !self.facts().broken().is_empty()
    }

    /// At launch: opens what is needed, then keeps watching.
    pub fn start(&self) -> Result<(), String> {
        self.watch();
        self.shared.models.set(self.shared.parts.models());
        if !steps_to_show(self.finished(), &self.facts()).is_empty() {
            self.open(Opening::Foreground)?;
        }
        self.look();
        Ok(())
    }

    /// Before any window opens: opening is what sets the faster rate.
    fn watch(&self) {
        if self.shared.watching.replace(true) {
            return;
        }
        let host = self.weak();
        self.shared.timer.every(
            LOOK_IDLE,
            Box::new(move || {
                if let Some(shared) = host.upgrade() {
                    OnboardingHost { shared }.look();
                }
            }),
        );
    }

    /// Opens the window on what needs doing. The tray comes here too, so there's always a way back
    /// to it. With nothing to do it doesn't open.
    pub fn open(&self, how: Opening) -> Result<(), String> {
        if let Some(open) = &*self.shared.open.borrow() {
            open.show()?;
            if how == Opening::Foreground {
                open.bring_to_front();
            }
            return Ok(());
        }
        self.watch();
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
        let host = self.weak();
        let clock = Rc::clone(&self.shared.clock);
        let on_done: Rc<dyn Fn()> = Rc::new(move || {
            // Not from inside the window's own callback: closing drops the window.
            let host = host.clone();
            clock.later(
                Duration::ZERO,
                Box::new(move || {
                    if let Some(shared) = host.upgrade() {
                        OnboardingHost { shared }.close();
                    }
                }),
            );
        });
        let walk = Walk {
            steps,
            first_run: !finished,
            facts,
        };
        let window = (self.shared.make)(self.shared.parts.clone(), walk, Rc::clone(&on_done))?;
        window.on_close(Box::new(move || on_done()));
        window.start();
        window.show()?;
        if how == Opening::Foreground {
            window.bring_to_front();
        }
        *self.shared.open.borrow_mut() = Some(window);
        self.shared.timer.set_interval(LOOK_OPEN);
        Ok(())
    }

    /// Drops the window and everything it holds, then returns the freed memory to the OS.
    pub fn close(&self) {
        let window = self.shared.open.borrow_mut().take();
        if let Some(window) = window {
            window.hide();
        }
        self.shared.timer.set_interval(LOOK_IDLE);
        *self.shared.dismissed.borrow_mut() = self.facts().broken();
        self.look();
        self.shared
            .clock
            .later(WINDOW_TEARDOWN, Box::new(memory::release_freed));
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
            open.onboarding().look(facts);
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
            open.onboarding().on_flow_event(event);
        }
    }
}
