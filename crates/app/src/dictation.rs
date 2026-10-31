//! Connects the dictation flow to the windows: its events to the pill, and the pill menu to the
//! settings. The rest of what surrounds the flow is in `wf_shell::flow`.

use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};

use slint::Weak;
use wf_core::settings::{Settings, SettingsStore};
use wf_dictation::{Dictation, Event};
use wf_models::Catalog;
use wf_shell::flow::{self, rank_first};
use wf_shell::settings::values::{TIERS, choose_tier};

use crate::Tray;
use crate::meetings_window::{MeetingsBoard, MeetingsHost};
use crate::memory;
use crate::onboarding::FlowWatcher;
use crate::pill::{LANGUAGES, PillController, PillEvent};
use crate::settings_window::SettingsHost;

pub use wf_shell::flow::KeyWatch;

/// Starts the flow on the real machine. Its events reach `pill` and `tray` on the UI thread.
pub fn start(
    settings: Settings,
    pill: &PillController,
    tray: &Tray,
    board: &MeetingsBoard,
    watcher: FlowWatcher,
) -> Dictation {
    let on_event = deliver_to(pill, tray.as_weak(), board.clone(), watcher);
    flow::start(settings, on_event)
}

/// Returns the callback for the flow's threads. Events wait in a queue for the UI thread.
pub fn deliver_to(
    pill: &PillController,
    tray: Weak<Tray>,
    board: MeetingsBoard,
    watcher: FlowWatcher,
) -> Arc<dyn Fn(Event) + Send + Sync> {
    let (events, inbox) = mpsc::channel();
    let target = pill.clone();
    pill.on_events_arrived(move || {
        while let Ok(event) = inbox.try_recv() {
            if let Some(watch) = &*watcher.borrow() {
                watch(&event);
            }
            show(&target, &tray, &board, event);
        }
    });
    let window = pill.window();
    let events = Mutex::new(events);
    Arc::new(move |event| {
        let events = events.lock().unwrap_or_else(PoisonError::into_inner);
        if events.send(event).is_ok() {
            let _ = window.upgrade_in_event_loop(|window| window.invoke_events_arrived());
        }
    })
}

fn show(pill: &PillController, tray: &Weak<Tray>, board: &MeetingsBoard, event: Event) {
    match event {
        Event::Pill(state) => pill.set_state(state),
        Event::Level(level) => pill.set_level(level),
        Event::Toast(text) => pill.show_toast(text),
        Event::WordLearned(word) => pill.learn_word(word),
        Event::Recent(transcripts) => pill.set_recent(&transcripts),
        Event::Unloaded => memory::release_freed(),
        Event::Meeting(active) => {
            pill.hide_meeting_card();
            if let Some(tray) = tray.upgrade() {
                tray.set_in_meeting(active);
            }
        }
        Event::SuggestMeeting(app) => pill.suggest_meeting(app),
        Event::NoTextField(text) => pill.offer_copy(text),
        Event::Meetings(news) => board.post(news),
        Event::Trial(_) | Event::TrialFailed { .. } | Event::Inserted(_) => {}
    }
}

/// What the pill's buttons and menu do.
pub struct PillActions {
    pub dictation: Dictation,
    pub pill: PillController,
    pub store: Arc<SettingsStore>,
    pub settings: SettingsHost,
    pub meetings: MeetingsHost,
    pub catalog: Arc<Catalog>,
    /// Applies changed settings to the running app.
    pub apply: Rc<dyn Fn(&Settings)>,
}

impl PillActions {
    pub fn handle(&self, event: PillEvent) {
        use wf_core::dictation::PillInput;
        match event {
            PillEvent::ToggleHandsFree => self.dictation.on_pill(PillInput::Click),
            PillEvent::Cancel => self.dictation.on_pill(PillInput::Cancel),
            PillEvent::Stop => self.dictation.on_pill(PillInput::Stop),
            PillEvent::PasteLast => self.dictation.paste_last(),
            PillEvent::PasteRecent(index) => self.dictation.paste_recent(index),
            PillEvent::SelectMicrophone(index) => {
                if let Some(id) = self.pill.microphone_id(index) {
                    self.change(|settings| rank_first(&mut settings.general.microphones, id));
                }
            }
            PillEvent::SelectLanguage(index) => {
                if let Some((_, code)) = LANGUAGES.get(index) {
                    self.change(|settings| {
                        settings.general.languages = code.iter().map(|&c| c.to_owned()).collect();
                    });
                }
            }
            PillEvent::SelectPipelineQuality(index) => {
                if let Some(&tier) = TIERS.get(index) {
                    let catalog = Arc::clone(&self.catalog);
                    self.change(|settings| choose_tier(settings, tier, &catalog));
                }
            }
            PillEvent::TakeNotes => self.dictation.toggle_meeting(),
            PillEvent::CopyText(text) => self.dictation.copy_text(text),
            PillEvent::OpenMeetings => {
                if let Err(error) = self.meetings.open() {
                    eprintln!("could not open meetings: {error}");
                }
            }
            PillEvent::OpenSettings => {
                if let Err(error) = self.settings.open() {
                    eprintln!("could not open settings: {error}");
                }
            }
            PillEvent::Quit => {
                let _ = slint::quit_event_loop();
            }
            PillEvent::LearnWord(word) => self.learn_word(&word),
        }
    }

    /// Goes through the same save as any other setting, so the dictionary page and the flow both
    /// see it at once. The toast is how the user finds out.
    fn learn_word(&self, word: &str) {
        let mut added = false;
        self.change(|settings| added = settings.style.add_word(word));
        if added {
            self.pill
                .show_toast(format!("Added “{word}” to the dictionary"));
        }
    }

    fn change(&self, change: impl FnOnce(&mut Settings)) {
        if let Err(error) = self.store.update(change) {
            self.pill
                .show_toast(format!("Couldn't save settings: {error}"));
        }
        (self.apply)(&self.store.get());
        self.settings.refresh();
    }
}
