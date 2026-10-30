//! Keeps the one open Meetings window, and nothing at all while it's closed.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use directories::UserDirs;
use slint::{CloseRequestResponse, ComponentHandle, PlatformError, Timer, Weak};
use wf_core::settings::{SettingsStore, data_dir};
use wf_dictation::{Dictation, MeetingNews};
use wf_meetings::MeetingStore;
use wf_platform::desktop;

use super::{MeetingsController, MeetingsParts};
use crate::settings_window::values::theme_index;
use crate::{MeetingsWindow, memory};

/// Long enough for the OS window to be gone once the controller is dropped.
const WINDOW_TEARDOWN: Duration = Duration::from_millis(600);

/// Builds what a window needs, each time one opens.
pub type MakeParts = Rc<dyn Fn() -> MeetingsParts>;

type Open = Rc<RefCell<Option<MeetingsController>>>;

/// Where the flow's meeting news goes: to the open window, or nowhere. Made before the flow, which
/// the host needs, so the two can meet.
#[derive(Clone, Default)]
pub struct MeetingsBoard {
    open: Rc<RefCell<Option<Open>>>,
}

impl MeetingsBoard {
    pub fn post(&self, news: MeetingNews) {
        if let Some(open) = &*self.open.borrow()
            && let Some(controller) = &*open.borrow()
        {
            controller.news(news);
        }
    }
}

#[derive(Clone)]
pub struct MeetingsHost {
    make_parts: MakeParts,
    open: Open,
}

impl MeetingsHost {
    pub fn new(make_parts: MakeParts, board: &MeetingsBoard) -> Self {
        let open = Open::default();
        *board.open.borrow_mut() = Some(Rc::clone(&open));
        Self { make_parts, open }
    }

    /// The host of the running app, on the real data folder.
    pub fn system(
        settings: Arc<SettingsStore>,
        dictation: Dictation,
        board: &MeetingsBoard,
    ) -> Self {
        let data = data_dir().unwrap_or_else(|| PathBuf::from("wispr-free"));
        let export_dir = UserDirs::new()
            .and_then(|user| {
                let downloads = user.download_dir().map(PathBuf::from);
                downloads.or_else(|| Some(user.home_dir().to_path_buf()))
            })
            .unwrap_or_else(|| data.clone());
        let make_parts: MakeParts = Rc::new(move || MeetingsParts {
            store: MeetingStore::new(data.join("meetings")),
            actions: Rc::new(dictation.clone()),
            theme: theme_index(&settings.get()),
            export_dir: export_dir.clone(),
            bring_to_front: Rc::new(desktop::activate_app),
        });
        Self::new(make_parts, board)
    }

    pub fn is_open(&self) -> bool {
        self.open.borrow().is_some()
    }

    /// The open window, for whoever needs to reach it.
    pub fn window(&self) -> Option<Weak<MeetingsWindow>> {
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
        let controller = MeetingsController::new((self.make_parts)())?;
        let host = self.clone();
        controller.window().window().on_close_requested(move || {
            // Not from inside the window's own callback: the close drops the window.
            let host = host.clone();
            Timer::single_shot(Duration::ZERO, move || host.close());
            CloseRequestResponse::HideWindow
        });
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
