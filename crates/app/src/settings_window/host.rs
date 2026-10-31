//! Keeps the one open Settings window, and nothing at all while it's closed.

use std::rc::Rc;
use std::sync::Arc;

use slint::{CloseRequestResponse, ComponentHandle, Weak};
use wf_core::settings::SettingsStore;
use wf_dictation::Dictation;
use wf_shell::windows::{Window, WindowHost};

use super::{OnChange, SettingsController, SettingsParts, system_parts};
use crate::slint_clock::SlintClock;
use crate::{SettingsWindow, folder_picker};

/// Builds what a window needs, each time one opens.
pub type MakeParts = Rc<dyn Fn() -> SettingsParts>;

impl Window for SettingsController {
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
        self.start_probes();
    }
}

#[derive(Clone)]
pub struct SettingsHost {
    host: WindowHost<SettingsController>,
}

impl SettingsHost {
    pub fn new(make_parts: MakeParts) -> Self {
        let make = Rc::new(move || {
            SettingsController::new(make_parts()).map_err(|error| error.to_string())
        });
        Self {
            host: WindowHost::new(make, Rc::new(SlintClock)),
        }
    }

    /// Onboarding shares the models folder with this window, and downloads don't lock their
    /// files, so while it's open it comes forward in this window's place.
    pub fn redirect_with(&self, redirect: impl Fn() -> bool + 'static) {
        self.host.redirect_with(redirect);
    }

    /// The host of the running app, on the real OS and data folder.
    pub fn system(store: Arc<SettingsStore>, on_change: OnChange, dictation: Dictation) -> Self {
        Self::new(Rc::new(move || {
            system_parts(
                Arc::clone(&store),
                Rc::clone(&on_change),
                dictation.clone(),
                folder_picker(),
            )
        }))
    }

    pub fn is_open(&self) -> bool {
        self.host.is_open()
    }

    /// The open window, for whoever needs to reach it.
    pub fn window(&self) -> Option<Weak<SettingsWindow>> {
        self.host
            .with_window(|controller| controller.window().as_weak())
    }

    /// Shows the settings again in an open window, after something else changed them.
    pub fn refresh(&self) {
        self.host.with_window(|controller| controller.refresh());
    }

    /// Opens the window, or brings it forward if it's open already.
    pub fn open(&self) -> Result<(), String> {
        self.host.open()
    }

    /// Drops the window and everything it holds, then returns the freed memory to the OS.
    pub fn close(&self) {
        self.host.close();
    }
}
