//! Onboarding behind `Shell`. The host lives on the pages' thread, beside the pages it shows, and
//! the window is Swift's: this tells Swift what to do with it and hears back when it's gone.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wf_dictation::Event as FlowEvent;

use super::host::{OnboardingHost, OnboardingWindow, Opening, Walk};
use super::{Machine, Onboarding, OnboardingParts, SetupPage, Shown};
use crate::clock::Clock;
use crate::settings::{Pages, SettingsParts};
use crate::windows::Window;

/// What onboarding wants of the app.
#[derive(Debug, Clone, PartialEq)]
pub enum SetupCall {
    Show(Box<SetupPage>),
    Level(f32),
    /// Put the window up, leaving the focus where it is.
    Open,
    Raise,
    /// Close the window, and say so as any close does.
    Close,
    /// Nothing is open any more, as far as the rule between windows goes.
    Gone,
}

/// What the machine tells the rest of the run.
pub struct MachineHooks {
    pub set_setup_needed: Box<dyn Fn(bool) + Send>,
    pub close_settings: Box<dyn Fn() + Send>,
    /// Ends the run the way the menu does.
    pub quit: Box<dyn Fn() + Send>,
}

/// Makes the machine on the pages' thread, where its timers and `Rc`s can live.
pub type MakeMachine = Box<dyn FnOnce(&dyn Clock, MachineHooks) -> Rc<dyn Machine> + Send>;

/// What onboarding is built from, kept until the pages' thread starts.
pub struct SetupRecipe {
    pub make_machine: MakeMachine,
    pub hooks: MachineHooks,
    pub front: Box<dyn Fn(SetupCall) + Send>,
}

type Front = Rc<dyn Fn(SetupCall)>;

/// One open window, as Rust sees it.
pub struct SwiftWindow {
    onboarding: Rc<Onboarding>,
    front: Front,
    /// Closes of ours that Swift hasn't reported yet.
    unreported: Rc<Cell<u32>>,
    on_screen: Cell<bool>,
    /// Swift's window has gone already, so there's nothing to tell it.
    gone: Cell<bool>,
}

impl Window for SwiftWindow {
    fn show(&self) -> Result<(), String> {
        if !self.on_screen.replace(true) {
            (self.front)(SetupCall::Open);
        }
        Ok(())
    }

    fn hide(&self) {
        self.onboarding.pages().window_closed();
        (self.front)(SetupCall::Gone);
        if !self.gone.get() {
            self.unreported.set(self.unreported.get() + 1);
            (self.front)(SetupCall::Close);
        }
    }

    fn bring_to_front(&self) {
        (self.front)(SetupCall::Raise);
    }

    /// Swift's report lands in [`SetupDesk::window_closed`], which closes the host straight off.
    fn on_close(&self, _close: Box<dyn Fn()>) {}

    /// The pages outlive a window, so what they know may have aged since the last one.
    fn start(&self) {
        let pages = self.onboarding.pages();
        pages.start_probes();
        pages.recheck_models_folder();
    }
}

impl OnboardingWindow for SwiftWindow {
    fn onboarding(&self) -> &Onboarding {
        &self.onboarding
    }
}

/// Onboarding, on the pages' thread.
pub struct SetupDesk {
    host: OnboardingHost<SwiftWindow>,
    last_page: Rc<RefCell<Option<SetupPage>>>,
    unreported: Rc<Cell<u32>>,
}

impl SetupDesk {
    pub fn new(
        recipe: SetupRecipe,
        settings: SettingsParts,
        pages: Rc<Pages>,
        clock: Rc<dyn Clock>,
    ) -> Self {
        let SetupRecipe {
            make_machine,
            hooks,
            front,
        } = recipe;
        let front: Front = Rc::new(front);
        let last_page: Rc<RefCell<Option<SetupPage>>> = Rc::default();
        let unreported: Rc<Cell<u32>> = Rc::default();
        let make = {
            let (last_page, unreported) = (Rc::clone(&last_page), Rc::clone(&unreported));
            Rc::new(
                move |parts: OnboardingParts, walk: Walk, on_done: Rc<dyn Fn()>| {
                    let view = page_view(&front, &last_page);
                    let onboarding = Onboarding::new(parts, Rc::clone(&pages), view, walk, on_done);
                    Ok(SwiftWindow {
                        onboarding,
                        front: Rc::clone(&front),
                        unreported: Rc::clone(&unreported),
                        on_screen: Cell::default(),
                        gone: Cell::default(),
                    })
                },
            )
        };
        let machine = make_machine(clock.as_ref(), hooks);
        let parts = OnboardingParts { settings, machine };
        Self {
            host: OnboardingHost::new(parts, make, clock),
            last_page,
            unreported,
        }
    }

    /// Starts watching what would bring the window up, and opens it if it's needed now.
    pub fn watch(&self) {
        if let Err(error) = self.host.start() {
            eprintln!("could not open onboarding: {error}");
        }
    }

    /// Someone asked for it. With nothing to do it doesn't open.
    pub fn open(&self) {
        if let Err(error) = self.host.open(Opening::Foreground) {
            eprintln!("could not open onboarding: {error}");
        }
    }

    /// Swift's window is gone. Each close comes back here once, in order, so a report that a close
    /// of ours caused can't be taken for the user closing a window opened since.
    pub fn window_closed(&self) {
        let unreported = self.unreported.get();
        if unreported > 0 {
            self.unreported.set(unreported - 1);
        } else if self.host.is_open() {
            self.host.with_window(|open| open.gone.set(true));
            self.host.close();
        }
    }

    pub fn on_flow_event(&self, event: &FlowEvent) {
        self.host.on_flow_event(event);
    }

    /// What the open window shows.
    pub fn page(&self) -> Option<SetupPage> {
        let page = self.last_page.borrow();
        page.clone().filter(|_| self.host.is_open())
    }

    /// Runs `use_it` on the open window's walk.
    pub fn with_window(&self, use_it: impl FnOnce(&Onboarding)) {
        self.host.with_window(|open| use_it(open.onboarding()));
    }
}

/// Says each new page once: a re-check runs every second, and nothing need cross for it.
fn page_view(front: &Front, last: &Rc<RefCell<Option<SetupPage>>>) -> super::View {
    let (front, last) = (Rc::clone(front), Rc::clone(last));
    Rc::new(move |shown| match shown {
        Shown::TrialLevel(level) => front(SetupCall::Level(level)),
        Shown::Page(page) => {
            if last.borrow().as_ref() != Some(&*page) {
                *last.borrow_mut() = Some((*page).clone());
                front(SetupCall::Show(page));
            }
        }
    })
}
