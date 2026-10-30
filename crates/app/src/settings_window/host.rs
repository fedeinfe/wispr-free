//! Keeps the one open Settings window, and nothing at all while it's closed.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use directories::UserDirs;
use slint::{CloseRequestResponse, ComponentHandle, PlatformError, Timer, Weak};
use wf_core::settings::{SettingsStore, data_dir};
use wf_core::shortcut::Platform;
use wf_dictation::Dictation;
use wf_models::Catalog;

use super::system::SystemDesktop;
use super::{OnChange, Paths, SettingsController, SettingsParts};
use crate::{SettingsWindow, memory};

/// Long enough for the OS window to be gone once the controller is dropped. The memory spike on
/// macOS collected this long after closing.
const WINDOW_TEARDOWN: Duration = Duration::from_millis(600);

/// Builds what a window needs, each time one opens.
pub type MakeParts = Rc<dyn Fn() -> SettingsParts>;

/// Asked before the window opens. True means something else took the request.
pub type Redirect = Rc<RefCell<Option<Box<dyn Fn() -> bool>>>>;

#[derive(Clone)]
pub struct SettingsHost {
    make_parts: MakeParts,
    open: Rc<RefCell<Option<SettingsController>>>,
    redirect: Redirect,
}

impl SettingsHost {
    pub fn new(make_parts: MakeParts) -> Self {
        Self {
            make_parts,
            open: Rc::default(),
            redirect: Rc::default(),
        }
    }

    /// Onboarding shares the models folder with this window, and downloads don't lock their
    /// files, so while it's open it comes forward in this window's place.
    pub fn redirect_with(&self, redirect: impl Fn() -> bool + 'static) {
        *self.redirect.borrow_mut() = Some(Box::new(redirect));
    }

    /// The host of the running app, on the real OS and data folder.
    pub fn system(store: Arc<SettingsStore>, on_change: OnChange, dictation: Dictation) -> Self {
        Self::new(Rc::new(move || {
            system_parts(Arc::clone(&store), Rc::clone(&on_change), dictation.clone())
        }))
    }

    pub fn is_open(&self) -> bool {
        self.open.borrow().is_some()
    }

    /// The open window, for whoever needs to reach it.
    pub fn window(&self) -> Option<Weak<SettingsWindow>> {
        let open = self.open.borrow();
        open.as_ref()
            .map(|controller| controller.window().as_weak())
    }

    /// Shows the settings again in an open window, after something else changed them.
    pub fn refresh(&self) {
        if let Some(controller) = &*self.open.borrow() {
            controller.refresh();
        }
    }

    /// Opens the window, or brings it forward if it's open already.
    pub fn open(&self) -> Result<(), PlatformError> {
        if self.redirect.borrow().as_ref().is_some_and(|taken| taken()) {
            return Ok(());
        }
        if let Some(controller) = &*self.open.borrow() {
            controller.window().show()?;
            controller.bring_to_front();
            return Ok(());
        }
        let controller = SettingsController::new((self.make_parts)())?;
        let host = self.clone();
        controller.window().window().on_close_requested(move || {
            // Not from inside the window's own callback: the close drops the window.
            let host = host.clone();
            Timer::single_shot(Duration::ZERO, move || host.close());
            CloseRequestResponse::HideWindow
        });
        controller.start_probes();
        controller.window().show()?;
        controller.bring_to_front();
        *self.open.borrow_mut() = Some(controller);
        Ok(())
    }

    /// Drops the window and everything it holds, then returns the freed memory to the OS.
    pub fn close(&self) {
        if let Some(controller) = self.open.borrow_mut().take() {
            let _ = controller.window().hide();
        }
        Timer::single_shot(WINDOW_TEARDOWN, memory::release_freed);
    }
}

/// What a window on the real machine is built from.
pub fn system_parts(
    store: Arc<SettingsStore>,
    on_change: OnChange,
    dictation: Dictation,
) -> SettingsParts {
    let data_dir = data_dir().unwrap_or_else(|| PathBuf::from("wispr-free"));
    let export_dir = UserDirs::new()
        .and_then(|user| {
            let downloads = user.download_dir().map(PathBuf::from);
            downloads.or_else(|| Some(user.home_dir().to_path_buf()))
        })
        .unwrap_or_else(|| data_dir.clone());
    SettingsParts {
        store,
        catalog: Arc::new(Catalog::bundled()),
        desktop: Rc::new(SystemDesktop { dictation }),
        on_change,
        paths: Paths {
            data_dir,
            export_dir,
        },
        platform: Platform::current(),
    }
}
