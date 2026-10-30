//! The real [`Machine`].

use std::path::PathBuf;

use slint::Weak;
use wf_core::dictation::PillInput;
use wf_dictation::Dictation;
use wf_platform::desktop;
use wf_platform::permissions::{self, Permission, Permissions};

use super::Machine;
use crate::Tray;
use crate::dictation::KeyWatch;
use crate::settings_window::SettingsHost;

pub struct SystemMachine {
    pub bundle: PathBuf,
    pub dictation: Dictation,
    pub keys: KeyWatch,
    pub settings: SettingsHost,
    pub tray: Weak<Tray>,
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
        if let Some(tray) = self.tray.upgrade() {
            tray.set_setup_needed(needed);
        }
    }

    fn close_settings(&self) {
        self.settings.close();
    }

    fn reopen_app(&self) {
        if let Err(error) = desktop::reopen_after_quit(&self.bundle) {
            eprintln!("could not reopen the app: {error}");
            return;
        }
        let _ = slint::quit_event_loop();
    }
}
