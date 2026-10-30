//! Connects the dictation flow to the app: its events to the pill, the keyboard to the flow, and
//! the pill menu to the settings. The flow itself lives in `wf_dictation`.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use slint::{Timer, TimerMode, Weak};
use wf_core::settings::{PipelineTier, Settings, SettingsStore, data_dir};
use wf_dictation::{Dictation, Event, Parts};
use wf_history::History;
use wf_models::{Catalog, ModelStore};
use wf_platform::keys::{self, KeyListener};
use wf_platform::permissions::Permissions;

use crate::Tray;
use crate::debug::ClipRun;
use crate::meetings_window::{MeetingsBoard, MeetingsHost};
use crate::memory;
use crate::onboarding::FlowWatcher;
use crate::pill::{LANGUAGES, PillController, PillEvent};
use crate::settings_window::SettingsHost;
use crate::settings_window::values::choose_tier;

/// How often to try the keyboard again after the OS refused it. Permissions arrive at any time.
const KEYS_RETRY: Duration = Duration::from_secs(5);

const PIPELINE_QUALITIES: [PipelineTier; 4] = [
    PipelineTier::Lite,
    PipelineTier::Balanced,
    PipelineTier::High,
    PipelineTier::Custom,
];

/// Starts the flow on the real machine. Its events reach `pill` and `tray` on the UI thread.
pub fn start(
    settings: Settings,
    pill: &PillController,
    tray: &Tray,
    board: &MeetingsBoard,
    clip: Option<&ClipRun>,
    watcher: FlowWatcher,
) -> Dictation {
    let data = data_dir().unwrap_or_else(|| PathBuf::from("wispr-free"));
    let default_models = ModelStore::default_root().unwrap_or_else(|| data.join("models"));
    let history = History::default_path().unwrap_or_else(|| data.join("history.sqlite3"));
    let meetings = data.join("meetings");
    let executable = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("wispr-free"));
    let on_event = deliver_to(pill, tray.as_weak(), board.clone(), watcher);
    let mut parts = Parts::system(
        settings,
        Arc::new(Catalog::bundled()),
        default_models,
        history,
        meetings,
        executable,
        on_event,
    );
    if let Some(clip) = clip {
        clip.fit(&mut parts);
    }
    Dictation::start(parts)
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
        Event::Recent(transcripts) => pill.set_recent(&transcripts),
        Event::Unloaded => memory::release_freed(),
        Event::Meeting(active) => {
            pill.hide_card();
            if let Some(tray) = tray.upgrade() {
                tray.set_in_meeting(active);
            }
        }
        Event::SuggestMeeting(app) => pill.suggest_meeting(app),
        Event::Meetings(news) => board.post(news),
        Event::Trial(_) | Event::TrialFailed { .. } | Event::Inserted(_) => {}
    }
}

/// Listens to the keyboard for as long as the returned value lives. While the OS refuses, the pill
/// says why and this keeps trying. The way a bare binary runs: in the app bundle, onboarding's
/// watch does all this through [`KeyWatch::refresh`].
pub fn listen_to_keys(dictation: &Dictation) -> KeyWatch {
    let watch = KeyWatch::idle(dictation);
    if !watch.try_listen_and_report() {
        let this = watch.clone();
        watch.retry.start(TimerMode::Repeated, KEYS_RETRY, move || {
            if this.try_listen_and_report() {
                this.retry.stop();
            }
        });
    }
    watch
}

#[derive(Clone)]
pub struct KeyWatch {
    dictation: Dictation,
    listener: Rc<RefCell<Option<KeyListener>>>,
    retry: Rc<Timer>,
    permissions: Rc<Cell<Option<Permissions>>>,
}

impl KeyWatch {
    /// Not listening yet.
    pub fn idle(dictation: &Dictation) -> Self {
        Self {
            dictation: dictation.clone(),
            listener: Rc::default(),
            retry: Rc::new(Timer::default()),
            permissions: Rc::default(),
        }
    }

    pub fn is_listening(&self) -> bool {
        self.listener.borrow().is_some()
    }

    /// Keeps the listener in step with the permissions. Any change restarts it: a tap made
    /// without Accessibility can't swallow keys until it's made again, and one made before a
    /// revoke may be dead with nothing to say so. `may_prompt` false holds back the first try,
    /// which could bring up an OS question.
    pub fn refresh(&self, permissions: &Permissions, may_prompt: bool) {
        if self.permissions.replace(Some(*permissions)) != Some(*permissions) {
            self.listener.borrow_mut().take();
        }
        if may_prompt && !self.is_listening() {
            let _ = self.try_listen();
        }
    }

    fn try_listen(&self) -> Result<(), keys::KeysError> {
        let flow = self.dictation.clone();
        let listener = keys::listen(move |signal| flow.on_key(signal))?;
        *self.listener.borrow_mut() = Some(listener);
        Ok(())
    }

    fn try_listen_and_report(&self) -> bool {
        match self.try_listen() {
            Ok(()) => {
                self.dictation.set_blocked(None);
                true
            }
            Err(error) => {
                self.dictation.set_blocked(Some(error.to_string()));
                false
            }
        }
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
                if let Some(&tier) = PIPELINE_QUALITIES.get(index) {
                    let catalog = Arc::clone(&self.catalog);
                    self.change(|settings| choose_tier(settings, tier, &catalog));
                }
            }
            PillEvent::TakeNotes => self.dictation.toggle_meeting(),
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

/// Puts `id` at the top of the microphone ranking, keeping the rest in order.
fn rank_first(ranking: &mut Vec<String>, id: String) {
    ranking.retain(|known| *known != id);
    ranking.insert(0, id);
}

#[cfg(test)]
mod tests {
    use super::rank_first;

    #[test]
    fn a_chosen_microphone_moves_to_the_top_of_the_ranking() {
        let mut ranking = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];

        rank_first(&mut ranking, "c".to_owned());
        assert_eq!(ranking, ["c", "a", "b"]);

        rank_first(&mut ranking, "new".to_owned());
        assert_eq!(ranking, ["new", "c", "a", "b"]);
    }
}
