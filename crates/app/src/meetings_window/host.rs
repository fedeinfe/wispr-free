//! Keeps the one open Meetings window, and nothing at all while it's closed.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use slint::{CloseRequestResponse, ComponentHandle, Weak};
use wf_core::settings::SettingsStore;
use wf_dictation::{Dictation, MeetingNews};
use wf_shell::meetings::{system_folders, system_parts};
use wf_shell::windows::{Window, WindowHost};

use super::{MeetingsController, MeetingsParts};
use crate::MeetingsWindow;
use crate::slint_clock::SlintClock;

/// Builds what a window needs, each time one opens.
pub type MakeParts = Rc<dyn Fn() -> MeetingsParts>;

impl Window for MeetingsController {
    fn show(&self) -> Result<(), String> {
        self.window().show().map_err(|error| error.to_string())
    }

    fn hide(&self) {
        let _ = self.window().hide();
    }

    fn bring_to_front(&self) {
        (**self).bring_to_front();
    }

    fn on_close(&self, close: Box<dyn Fn()>) {
        self.window().window().on_close_requested(move || {
            close();
            CloseRequestResponse::HideWindow
        });
    }
}

/// Where the flow's meeting news goes: to the open window, or nowhere. Made before the flow, which
/// the host needs, so the two can meet.
#[derive(Clone, Default)]
pub struct MeetingsBoard {
    host: Rc<RefCell<Option<WindowHost<MeetingsController>>>>,
}

impl MeetingsBoard {
    pub fn post(&self, news: MeetingNews) {
        if let Some(host) = &*self.host.borrow() {
            host.with_window(|controller| controller.news(news));
        }
    }
}

#[derive(Clone)]
pub struct MeetingsHost {
    host: WindowHost<MeetingsController>,
}

impl MeetingsHost {
    pub fn new(make_parts: MakeParts, board: &MeetingsBoard) -> Self {
        let make = Rc::new(move || {
            MeetingsController::new(make_parts()).map_err(|error| error.to_string())
        });
        let host = WindowHost::new(make, Rc::new(SlintClock));
        *board.host.borrow_mut() = Some(host.clone());
        Self { host }
    }

    /// The host of the running app, on the real data folder.
    pub fn system(dictation: Dictation, board: &MeetingsBoard) -> Self {
        let (data, export_dir) = system_folders();
        let make_parts: MakeParts =
            Rc::new(move || system_parts(&data, &export_dir, 0, dictation.clone()));
        Self::new(make_parts, board)
    }

    pub fn is_open(&self) -> bool {
        self.host.is_open()
    }

    /// The open window, for whoever needs to reach it.
    pub fn window(&self) -> Option<Weak<MeetingsWindow>> {
        self.host
            .with_window(|controller| controller.window().as_weak())
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
