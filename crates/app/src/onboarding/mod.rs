//! First-run onboarding (ticket 13): the window is created on open and dropped on close, and it
//! comes back only for a step that broke. [`OnboardingController`] binds the .slint window to an
//! [`Onboarding`], which holds everything it knows and does.

mod host;

use std::ops::Deref;
use std::rc::Rc;

use slint::{ComponentHandle, Model, ModelRc, PlatformError, VecModel};
use wf_platform::permissions::Access;
use wf_shell::onboarding::trial::{self, Check, Phase};
use wf_shell::onboarding::{Onboarding, SetupPage, Shown, StepRow, Walk};

use crate::settings_window::{Controller, Surface};
use crate::{Consent, Mark, OnboardingWindow, Prefs, Setup, SetupStep, TrialPhase};

pub use host::{OnboardingHost, system_machine};
pub use wf_shell::onboarding::{FlowWatcher, Machine, OnboardingParts, Opening, steps};

crate::same_fields!(StepRow => SetupStep { title, page, done });

impl Surface for OnboardingWindow {
    fn create() -> Result<Self, PlatformError> {
        Self::new()
    }

    fn prefs(&self) -> Prefs<'_> {
        self.global::<Prefs>()
    }
}

fn consent(access: Access) -> Consent {
    match access {
        Access::Granted => Consent::Granted,
        Access::Denied => Consent::Denied,
        Access::Unknown => Consent::NotAsked,
    }
}

fn mark(check: Check) -> Mark {
    match check {
        Check::Waiting => Mark::Waiting,
        Check::Passed => Mark::Passed,
        Check::Failed => Mark::Failed,
    }
}

/// One open onboarding window. Dropping it stops everything the window started.
pub struct OnboardingController {
    onboarding: Rc<Onboarding>,
    pages: Controller<OnboardingWindow>,
}

/// Connects a `Setup` callback to a method of [`Onboarding`] through a weak reference.
macro_rules! wire {
    ($onboarding:ident, $setup:ident . $on:ident, |$this:ident $(, $arg:ident)*| $body:expr) => {{
        let weak = Rc::downgrade($onboarding);
        $setup.$on(move |$($arg),*| {
            if let Some($this) = weak.upgrade() {
                $body
            }
        });
    }};
}

impl OnboardingController {
    pub fn new(
        parts: OnboardingParts,
        walk: Walk,
        on_done: Rc<dyn Fn()>,
    ) -> Result<Self, PlatformError> {
        let pages = Controller::<OnboardingWindow>::new(parts.settings.clone())?;
        let weak = pages.window().as_weak();
        let view = Rc::new(move |shown| {
            if let Some(window) = weak.upgrade() {
                show(&window.global::<Setup>(), shown);
            }
        });
        let onboarding = Onboarding::new(parts, Rc::clone(pages.pages()), view, walk, on_done);
        connect(&pages.window().global::<Setup>(), &onboarding);
        Ok(Self { onboarding, pages })
    }

    pub fn window(&self) -> &OnboardingWindow {
        self.pages.window()
    }

    pub fn pages(&self) -> &Controller<OnboardingWindow> {
        &self.pages
    }
}

impl Deref for OnboardingController {
    type Target = Onboarding;

    fn deref(&self) -> &Onboarding {
        &self.onboarding
    }
}

fn connect(setup: &Setup<'_>, onboarding: &Rc<Onboarding>) {
    wire!(onboarding, setup.on_back, |this| this.back());
    wire!(onboarding, setup.on_continue, |this| this.forward());
    wire!(onboarding, setup.on_ask, |this, index| this.ask(index));
    wire!(onboarding, setup.on_open_pane, |this, index| this
        .open_pane(index));
    wire!(onboarding, setup.on_reopen_app, |this| this.reopen_app());
    wire!(onboarding, setup.on_try_with_mouse, |this| this
        .try_with_mouse());
    wire!(onboarding, setup.on_stop_trial, |this| this.stop_trial());
}

fn show(setup: &Setup<'_>, shown: Shown) {
    match shown {
        Shown::TrialLevel(level) => setup.set_trial_level(level),
        Shown::Page(page) => show_page(setup, *page),
    }
}

fn show_page(setup: &Setup<'_>, page: SetupPage) {
    let rows: Vec<SetupStep> = page.steps.into_iter().map(SetupStep::from).collect();
    // This runs every second: a new model would rebuild the sidebar each time.
    if !setup.get_steps().iter().eq(rows.iter().cloned()) {
        setup.set_steps(ModelRc::new(VecModel::from(rows)));
    }
    setup.set_current(page.current);
    setup.set_page(page.page);
    setup.set_can_continue(page.can_continue);
    setup.set_continue_label(page.continue_label.into());
    setup.set_microphone(consent(page.microphone));
    setup.set_accessibility(consent(page.accessibility));
    setup.set_input_monitoring(consent(page.input_monitoring));
    setup.set_microphone_looked(page.microphone_looked);
    setup.set_microphone_name(page.microphone_name.into());
    setup.set_keys_stuck(page.keys_stuck);
    setup.set_models_ready(page.models_ready);
    setup.set_shortcut(page.shortcut.into());
    show_trial(setup, page.trial);
}

fn show_trial(setup: &Setup<'_>, trial: trial::View) {
    setup.set_trial_phase(match trial.phase {
        Phase::Idle => TrialPhase::Idle,
        Phase::Listening => TrialPhase::Listening,
        Phase::Working => TrialPhase::Working,
    });
    setup.set_trial_keys(mark(trial.keys));
    setup.set_trial_microphone(mark(trial.microphone));
    setup.set_trial_models(mark(trial.models));
    setup.set_trial_message(trial.message.into());
    setup.set_trial_problem(trial.problem);
    setup.set_trial_heard(trial.heard.unwrap_or_default().into());
    setup.set_trial_can_stop(trial.can_stop);
}
