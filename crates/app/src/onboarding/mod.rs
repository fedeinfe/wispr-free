//! First-run onboarding (ticket 13): microphone, keyboard and paste, models, a trial dictation.
//! The window is created on open and dropped on close, and it comes back only for a step that
//! broke. [`OnboardingController`] is one open window. It leaves models, shortcuts and the rest of
//! `Prefs` to a [`Controller`] of the Settings pages, and adds the steps and the trial.

mod host;
mod system;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use slint::{ComponentHandle, Model, ModelRc, PlatformError, SharedString, VecModel};
use wf_core::pill::PillState;
use wf_core::settings::{PipelineTier, Settings, SettingsStore};
use wf_core::shortcut::{Action, Platform};
use wf_dictation::Event as FlowEvent;
use wf_dictation::plan::{plan, preset_tier};
use wf_models::{Catalog, ModelStatus, ModelStore};
use wf_platform::permissions::{Access, Permission, Permissions, settings_link};

use crate::settings_window::{Controller, SettingsParts, Surface};
use crate::{Consent, Mark, OnboardingWindow, Prefs, Setup, SetupStep, TrialPhase};
use wf_shell::onboarding::steps::{Facts, Hardware, Models, Step, first_open};
use wf_shell::onboarding::trial::{self, Check, Phase, Trial};
use wf_shell::settings::values::choose_tier;

pub use host::{FlowWatcher, OnboardingHost, Opening};
pub use system::SystemMachine;
pub use wf_shell::onboarding::steps;

/// Looks with both permissions on and no keys before suggesting a reopen. One look a second, so
/// that the key listener has had its chance.
const STUCK_LOOKS: u32 = 8;

/// What onboarding needs from the OS and the running flow, behind a seam so that every state of
/// every permission can be tested without touching the real ones.
pub trait Machine {
    fn permissions(&self) -> Permissions;
    /// Brings up the OS's own question.
    fn ask(&self, permission: Permission);
    /// Whether the app is seeing the keyboard.
    fn keys_listening(&self) -> bool;
    /// A chance to start or restart the key listener. `permissions` are the latest; `may_prompt`
    /// says whether starting it may bring up an OS question.
    fn refresh_keys(&self, permissions: &Permissions, may_prompt: bool);
    fn arm_trial(&self, armed: bool);
    fn start_trial(&self);
    fn stop_trial(&self);
    fn set_blocked(&self, reason: Option<String>);
    /// Whether the tray offers to finish the setup.
    fn set_setup_needed(&self, needed: bool);
    /// The models step shares its files with the Settings window.
    fn close_settings(&self);
    fn reopen_app(&self);
}

impl Surface for OnboardingWindow {
    fn create() -> Result<Self, PlatformError> {
        Self::new()
    }

    fn prefs(&self) -> Prefs<'_> {
        self.global::<Prefs>()
    }
}

/// Everything onboarding is built from.
#[derive(Clone)]
pub struct OnboardingParts {
    pub settings: SettingsParts,
    pub machine: Rc<dyn Machine>,
}

impl OnboardingParts {
    pub fn store(&self) -> &Arc<SettingsStore> {
        &self.settings.store
    }

    /// Whether the models of the settings in force are there, by the same rule dictation uses.
    pub fn models(&self) -> Models {
        let settings = self.settings.store.get();
        let default_root = self.settings.paths.data_dir.join("models");
        let store =
            ModelStore::configured(settings.pipeline.models_folder.as_deref(), &default_root);
        models_state(&settings, &self.settings.catalog, &store)
    }
}

fn models_state(settings: &Settings, catalog: &Catalog, store: &ModelStore) -> Models {
    if !store.available() {
        return Models::FolderGone;
    }
    match plan(settings, catalog, store) {
        Ok(plan) if plan.llm_problem.is_none() => Models::Ready,
        _ => Models::Missing,
    }
}

/// Picks the recommended tier for someone who hasn't chosen: the tier is still the default and
/// nothing of it is downloaded.
fn tier_to_recommend(
    settings: &Settings,
    catalog: &Catalog,
    store: &ModelStore,
    recommended: &str,
) -> Option<PipelineTier> {
    let wanted = match recommended {
        "Lite" => PipelineTier::Lite,
        "Balanced" => PipelineTier::Balanced,
        "High" => PipelineTier::High,
        _ => return None,
    };
    let current = settings.pipeline.tier;
    let current_models = preset_tier(current).map(|tier| catalog.models_for_tier(tier));
    let untouched = current == PipelineTier::default()
        && current_models.is_some_and(|models| {
            models
                .iter()
                .all(|model| store.status(model) == ModelStatus::NotInstalled)
        });
    (untouched && wanted != current).then_some(wanted)
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

fn permission_at(index: i32) -> Option<Permission> {
    match index {
        0 => Some(Permission::Microphone),
        1 => Some(Permission::Accessibility),
        2 => Some(Permission::InputMonitoring),
        _ => None,
    }
}

fn page_of(step: Step) -> i32 {
    match step {
        Step::Microphone => 0,
        Step::Keys => 1,
        Step::Models => 2,
        Step::Trial => 3,
    }
}

/// One open onboarding window. Dropping it stops everything the window started.
pub struct OnboardingController {
    inner: Rc<Inner>,
}

struct Inner {
    pages: Controller<OnboardingWindow>,
    parts: OnboardingParts,
    /// This is the first run, so finishing it is what gets stored.
    first_run: bool,
    steps: RefCell<Vec<Step>>,
    current: Cell<usize>,
    /// The user moved between steps, so the window no longer picks one for them.
    moved: Cell<bool>,
    facts: Cell<Facts>,
    trial: RefCell<Trial>,
    keyless_looks: Cell<u32>,
    recommended: Cell<bool>,
    on_done: Rc<dyn Fn()>,
}

impl OnboardingController {
    /// A window for `steps`, opened at the first with something to do.
    pub fn new(
        parts: OnboardingParts,
        steps: Vec<Step>,
        first_run: bool,
        facts: Facts,
        on_done: Rc<dyn Fn()>,
    ) -> Result<Self, PlatformError> {
        let pages = Controller::<OnboardingWindow>::new(parts.settings.clone())?;
        let current = first_open(&steps, &facts);
        let inner = Rc::new(Inner {
            pages,
            parts,
            first_run,
            steps: RefCell::new(steps),
            current: Cell::new(current),
            moved: Cell::default(),
            facts: Cell::new(facts),
            trial: RefCell::default(),
            keyless_looks: Cell::default(),
            recommended: Cell::default(),
            on_done,
        });
        inner.connect();
        let weak = Rc::downgrade(&inner);
        inner.pages.on_microphones(move || {
            if let Some(inner) = weak.upgrade() {
                inner.on_microphones();
            }
        });
        inner.arrive();
        inner.show();
        Ok(Self { inner })
    }

    pub fn window(&self) -> &OnboardingWindow {
        self.inner.pages.window()
    }

    pub fn pages(&self) -> &Controller<OnboardingWindow> {
        &self.inner.pages
    }

    pub fn step(&self) -> Step {
        self.inner.step()
    }

    pub fn steps(&self) -> Vec<Step> {
        self.inner.steps.borrow().clone()
    }

    /// The whole picture again, from a re-check.
    pub fn look(&self, facts: Facts) {
        self.inner.look(facts);
    }

    /// The hardware, as this window last heard it.
    pub fn hardware(&self) -> Hardware {
        self.inner.hardware()
    }

    /// Something the running flow reported.
    pub fn on_flow_event(&self, event: &FlowEvent) {
        self.inner.on_flow_event(event);
    }
}

impl Drop for OnboardingController {
    fn drop(&mut self) {
        // Armed, the shortcut would never paste again.
        self.inner.parts.machine.arm_trial(false);
    }
}

/// Connects a `Setup` callback to a method of [`Inner`] through a weak reference.
macro_rules! wire {
    ($inner:ident, $setup:ident . $on:ident, |$this:ident $(, $arg:ident)*| $body:expr) => {{
        let weak = Rc::downgrade($inner);
        $setup.$on(move |$($arg),*| {
            if let Some($this) = weak.upgrade() {
                $body
            }
        });
    }};
}

impl Inner {
    fn setup(&self) -> Setup<'_> {
        self.pages.window().global::<Setup>()
    }

    fn step(&self) -> Step {
        self.steps.borrow()[self.current.get()]
    }

    fn connect(self: &Rc<Self>) {
        let inner = self;
        let setup = self.setup();
        wire!(inner, setup.on_back, |this| this.back());
        wire!(inner, setup.on_continue, |this| this.forward());
        wire!(inner, setup.on_ask, |this, index| {
            if let Some(permission) = permission_at(index) {
                this.parts.machine.ask(permission);
            }
        });
        wire!(inner, setup.on_open_pane, |this, index| {
            if let Some(link) = permission_at(index).and_then(settings_link) {
                let _ = this.parts.settings.desktop.open(link);
            }
        });
        wire!(inner, setup.on_reopen_app, |this| this
            .parts
            .machine
            .reopen_app());
        wire!(inner, setup.on_try_with_mouse, |this| {
            this.trial.borrow_mut().started_by_mouse();
            this.parts.machine.start_trial();
        });
        wire!(inner, setup.on_stop_trial, |this| this
            .parts
            .machine
            .stop_trial());
    }

    fn back(&self) {
        let current = self.current.get();
        if current > 0 {
            self.move_to(current - 1);
        }
    }

    fn forward(&self) {
        if !self.can_continue() {
            return;
        }
        let next = self.current.get() + 1;
        if next < self.steps.borrow().len() {
            self.move_to(next);
            return;
        }
        if self.first_run {
            let finished = self
                .parts
                .settings
                .store
                .update(|settings| settings.onboarding.finished = true);
            if let Err(error) = finished {
                let notice = format!("Couldn't save settings: {error}");
                self.pages.window().prefs().set_notice(notice.into());
                return;
            }
        }
        (self.on_done)();
    }

    fn move_to(&self, index: usize) {
        self.moved.set(true);
        self.current.set(index);
        self.arrive();
        self.show();
    }

    /// The window opens before the first look at the hardware is back. That first answer can
    /// settle where a resumed run belongs.
    fn on_microphones(&self) {
        let facts = self.facts.get();
        let first_answer = facts.hardware == Hardware::NotLookedYet;
        let facts = Facts {
            hardware: self.hardware(),
            ..facts
        };
        self.facts.set(facts);
        if first_answer && !self.moved.get() {
            self.current.set(first_open(&self.steps.borrow(), &facts));
            self.arrive();
        }
        self.show();
    }

    /// The trial listens to the real shortcut for as long as its step is showing.
    fn arrive(&self) {
        self.parts.machine.arm_trial(self.step() == Step::Trial);
        self.pages.look_for_microphones();
    }

    fn can_continue(&self) -> bool {
        let facts = self.facts.get();
        match self.step() {
            Step::Trial => self.trial_view(&facts).passed,
            step => facts.done(step),
        }
    }

    fn hardware(&self) -> Hardware {
        match self.pages.microphones() {
            None => Hardware::NotLookedYet,
            Some(devices) if devices.is_empty() => Hardware::NoMicrophone,
            Some(_) => Hardware::Microphone,
        }
    }

    fn look(&self, facts: Facts) {
        self.facts.set(Facts {
            hardware: self.hardware(),
            ..facts
        });
        self.keep_broken_steps();
        if matches!(self.step(), Step::Microphone | Step::Trial) {
            self.pages.look_for_microphones();
        }
        let stuck = facts.keys_allowed() && !facts.keys_listening;
        self.keyless_looks.set(if stuck {
            self.keyless_looks.get() + 1
        } else {
            0
        });
        // Only when it changed: a re-check rebuilds every row and clears the last error.
        let folder_gone = facts.models == Models::FolderGone;
        if folder_gone != self.pages.window().prefs().get_models_folder_missing() {
            self.pages.recheck_models_folder();
        }
        self.recommend_tier();
        self.show();
    }

    /// A fix in progress can meet a second break: it joins the steps instead of waiting for the
    /// window to close.
    fn keep_broken_steps(&self) {
        if self.first_run {
            return;
        }
        let facts = self.facts.get();
        let current = self.step();
        let mut steps = self.steps.borrow_mut();
        for step in facts.broken() {
            if !steps.contains(&step) {
                steps.push(step);
            }
        }
        steps.sort_by_key(|&step| page_of(step));
        let index = steps.iter().position(|&step| step == current);
        self.current.set(index.unwrap_or_default());
    }

    fn recommend_tier(&self) {
        if self.recommended.get() || !self.first_run {
            return;
        }
        let prefs = self.pages.window().prefs();
        let recommended = prefs.get_recommended_tier();
        if recommended.is_empty() {
            return;
        }
        self.recommended.set(true);
        let parts = &self.parts.settings;
        let default_root = parts.paths.data_dir.join("models");
        let settings = parts.store.get();
        let store =
            ModelStore::configured(settings.pipeline.models_folder.as_deref(), &default_root);
        if let Some(tier) =
            tier_to_recommend(&settings, &parts.catalog, &store, recommended.as_str())
        {
            let saved = parts
                .store
                .update(|settings| choose_tier(settings, tier, &parts.catalog));
            if saved.is_ok() {
                self.pages.refresh();
                (parts.on_change)(&parts.store.get());
            }
        }
    }

    fn shortcut(&self) -> String {
        let settings = self.parts.settings.store.get();
        let platform: Platform = self.parts.settings.platform;
        settings.shortcuts.get(Action::PushToTalk).label(platform)
    }

    fn trial_view(&self, facts: &Facts) -> trial::View {
        self.trial.borrow().view(facts, &self.shortcut())
    }

    fn on_flow_event(&self, event: &FlowEvent) {
        match event {
            FlowEvent::Pill(state) => {
                self.trial.borrow_mut().on_pill(state);
                if *state != PillState::ListeningPtt && *state != PillState::ListeningHandsFree {
                    self.setup().set_trial_level(0.0);
                }
            }
            FlowEvent::Level(level) => {
                self.trial.borrow_mut().on_level(*level);
                self.setup().set_trial_level(*level);
            }
            FlowEvent::Trial(text) => self.trial.borrow_mut().on_heard(text.clone()),
            FlowEvent::TrialFailed { part, reason } => {
                self.trial.borrow_mut().on_failed(*part, reason.clone());
            }
            _ => return,
        }
        self.show();
    }

    fn show(&self) {
        let facts = self.facts.get();
        let setup = self.setup();
        let trial = self.trial_view(&facts);
        let steps = self.steps.borrow();
        let rows: Vec<SetupStep> = steps
            .iter()
            .map(|&step| SetupStep {
                title: step.title().into(),
                page: page_of(step),
                done: if step == Step::Trial {
                    trial.passed
                } else {
                    facts.done(step)
                },
            })
            .collect();
        let last = self.current.get() + 1 == steps.len();
        drop(steps);
        // This runs every second: a new model would rebuild the sidebar each time.
        if !setup.get_steps().iter().eq(rows.iter().cloned()) {
            setup.set_steps(ModelRc::new(VecModel::from(rows)));
        }
        setup.set_current(self.current.get() as i32);
        setup.set_page(page_of(self.step()));
        setup.set_can_continue(self.can_continue());
        let label = match (last, self.first_run) {
            (false, _) => "Continue",
            (true, true) => "Finish",
            (true, false) => "Done",
        };
        setup.set_continue_label(label.into());

        let permissions = facts.permissions;
        setup.set_microphone(consent(permissions.microphone));
        setup.set_accessibility(consent(permissions.accessibility));
        setup.set_input_monitoring(consent(permissions.input_monitoring));
        let devices = self.pages.microphones();
        setup.set_microphone_looked(devices.is_some());
        let name = devices
            .and_then(|devices| devices.first().map(|device| device.name.clone()))
            .unwrap_or_default();
        setup.set_microphone_name(name.into());
        setup.set_keys_stuck(self.keyless_looks.get() >= STUCK_LOOKS);
        setup.set_models_ready(facts.models == Models::Ready);

        setup.set_shortcut(self.shortcut().into());
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
        setup.set_trial_heard(trial.heard.map_or_else(SharedString::new, Into::into));
        setup.set_trial_can_stop(trial.can_stop);
    }
}

#[cfg(test)]
mod tests {
    use wf_models::Tier;

    use super::*;

    fn store_with(root: &std::path::Path, catalog: &Catalog, ids: &[&str]) -> ModelStore {
        let store = ModelStore::new(root);
        for id in ids {
            let model = catalog.model(id).expect("in the catalog");
            let folder = store.model_dir(id);
            std::fs::create_dir_all(&folder).unwrap();
            for file in &model.files {
                let handle = std::fs::File::create(folder.join(&file.name)).unwrap();
                handle.set_len(file.size).unwrap();
            }
        }
        store
    }

    #[test]
    fn a_fresh_install_takes_the_recommended_tier() {
        let folder = tempfile::tempdir().unwrap();
        let catalog = Catalog::bundled();
        let store = store_with(folder.path(), &catalog, &[]);
        let settings = Settings::default();

        let pick = |name| tier_to_recommend(&settings, &catalog, &store, name);
        assert_eq!(pick("Lite"), Some(PipelineTier::Lite));
        assert_eq!(pick("High"), Some(PipelineTier::High));
        assert_eq!(pick("Balanced"), None, "already the tier");
        assert_eq!(pick("Unknown"), None);
    }

    #[test]
    fn a_chosen_tier_or_downloaded_models_stay() {
        let folder = tempfile::tempdir().unwrap();
        let catalog = Catalog::bundled();

        let mut chosen = Settings::default();
        chosen.pipeline.tier = PipelineTier::High;
        let empty = store_with(folder.path(), &catalog, &[]);
        assert_eq!(tier_to_recommend(&chosen, &catalog, &empty, "Lite"), None);

        let balanced = catalog.preset(Tier::Balanced);
        let store = store_with(folder.path(), &catalog, &[balanced.asr_model.as_str()]);
        let settings = Settings::default();
        assert_eq!(tier_to_recommend(&settings, &catalog, &store, "Lite"), None);
    }

    /// Ticket 14 moved Balanced from Parakeet to Whisper. Someone set up before that keeps their
    /// tier, and is told to fetch the new model rather than being moved to the recommended one.
    #[test]
    fn a_catalog_change_to_a_tier_leaves_the_tier_alone() {
        let folder = tempfile::tempdir().unwrap();
        let catalog = Catalog::bundled();
        let settings = Settings::default();
        let store = store_with(
            folder.path(),
            &catalog,
            &[
                "silero-vad-v5",
                "parakeet-tdt-0.6b-v3-int8",
                "qwen3-1.7b-q4_k_m",
            ],
        );

        assert_eq!(tier_to_recommend(&settings, &catalog, &store, "High"), None);
        assert_eq!(models_state(&settings, &catalog, &store), Models::Missing);
    }

    #[test]
    fn missing_models_and_a_gone_folder_read_differently() {
        let folder = tempfile::tempdir().unwrap();
        let catalog = Catalog::bundled();
        let settings = Settings::default();

        let store = ModelStore::new(folder.path());
        assert_eq!(models_state(&settings, &catalog, &store), Models::Missing);

        let gone = ModelStore::in_folder(folder.path().join("unplugged"));
        assert_eq!(models_state(&settings, &catalog, &gone), Models::FolderGone);

        let ids: Vec<_> = catalog
            .preset(Tier::Balanced)
            .model_ids()
            .map(str::to_owned)
            .collect();
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        let store = store_with(folder.path(), &catalog, &ids);
        assert_eq!(models_state(&settings, &catalog, &store), Models::Ready);
    }
}
