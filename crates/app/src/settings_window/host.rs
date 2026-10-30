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
use wf_models::{Catalog, ModelStore};

use super::system::SystemDesktop;
use super::{OnChange, Paths, SettingsController, SettingsParts};
use crate::{SettingsWindow, memory};

/// Long enough for the OS window to be gone once the controller is dropped. The memory spike on
/// macOS collected this long after closing.
const WINDOW_TEARDOWN: Duration = Duration::from_millis(600);

/// Builds what a window needs, each time one opens.
pub type MakeParts = Rc<dyn Fn() -> SettingsParts>;

#[derive(Clone)]
pub struct SettingsHost {
    make_parts: MakeParts,
    open: Rc<RefCell<Option<SettingsController>>>,
}

impl SettingsHost {
    pub fn new(make_parts: MakeParts) -> Self {
        Self {
            make_parts,
            open: Rc::default(),
        }
    }

    /// The host of the running app, on the real OS and data folder.
    pub fn system(store: Arc<SettingsStore>, on_change: OnChange) -> Self {
        Self::new(Rc::new(move || {
            system_parts(Arc::clone(&store), Rc::clone(&on_change))
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

    /// Opens the window, or brings it forward if it's open already.
    pub fn open(&self) -> Result<(), PlatformError> {
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

fn system_parts(store: Arc<SettingsStore>, on_change: OnChange) -> SettingsParts {
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
        models: Arc::new(ModelStore::new(data_dir.join("models"))),
        desktop: Rc::new(SystemDesktop),
        on_change,
        paths: Paths {
            data_dir,
            export_dir,
        },
        platform: Platform::current(),
    }
}
