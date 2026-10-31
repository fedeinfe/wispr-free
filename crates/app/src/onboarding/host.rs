//! Keeps the one open onboarding window. `wf_shell` decides when, and this builds it.

use std::ops::Deref;
use std::path::PathBuf;
use std::rc::Rc;

use slint::{CloseRequestResponse, ComponentHandle, Weak};
use wf_dictation::Dictation;
use wf_shell::flow::KeyWatch;
use wf_shell::onboarding::{Onboarding, OnboardingParts, OnboardingWindow, SystemMachine, Walk};
use wf_shell::windows::Window;

use super::OnboardingController;
use crate::Tray;
use crate::settings_window::SettingsHost;
use crate::slint_clock::SlintClock;

impl Window for OnboardingController {
    fn show(&self) -> Result<(), String> {
        self.window().show().map_err(|error| error.to_string())
    }

    fn hide(&self) {
        let _ = self.window().hide();
    }

    fn bring_to_front(&self) {
        self.pages().bring_to_front();
    }

    fn on_close(&self, close: Box<dyn Fn()>) {
        self.window().window().on_close_requested(move || {
            close();
            CloseRequestResponse::HideWindow
        });
    }

    fn start(&self) {
        self.pages().start_probes();
    }
}

impl OnboardingWindow for OnboardingController {
    fn onboarding(&self) -> &Onboarding {
        self
    }
}

#[derive(Clone)]
pub struct OnboardingHost {
    host: wf_shell::onboarding::OnboardingHost<OnboardingController>,
}

impl OnboardingHost {
    pub fn new(parts: OnboardingParts) -> Self {
        let make = Rc::new(|parts, walk: Walk, on_done| {
            OnboardingController::new(parts, walk, on_done).map_err(|error| error.to_string())
        });
        Self {
            host: wf_shell::onboarding::OnboardingHost::new(parts, make, Rc::new(SlintClock)),
        }
    }
}

impl Deref for OnboardingHost {
    type Target = wf_shell::onboarding::OnboardingHost<OnboardingController>;

    fn deref(&self) -> &Self::Target {
        &self.host
    }
}

/// The real `Machine`, with the tray and the run loop filled in.
pub fn system_machine(
    bundle: PathBuf,
    dictation: Dictation,
    keys: KeyWatch,
    settings: SettingsHost,
    tray: Weak<Tray>,
) -> SystemMachine {
    SystemMachine {
        bundle,
        dictation,
        keys,
        set_setup_needed: Box::new(move |needed| {
            if let Some(tray) = tray.upgrade() {
                tray.set_setup_needed(needed);
            }
        }),
        close_settings: Box::new(move || settings.close()),
        quit: Box::new(|| {
            let _ = slint::quit_event_loop();
        }),
    }
}
