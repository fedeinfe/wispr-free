//! What the app does around the dictation flow: starting it, and keeping the keyboard connected to
//! it. The flow itself lives in `wf_dictation`.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use wf_core::settings::{Settings, data_dir};
use wf_dictation::remote::engine_host_beside;
use wf_dictation::{Dictation, Event, Parts};
use wf_history::History;
use wf_models::{Catalog, ModelStore};
use wf_platform::keys::{self, KeyListener};
use wf_platform::permissions::Permissions;

use crate::clock::{Clock, Timer};
use crate::paths::Paths;

/// How often to try the keyboard again after the OS refused it. Permissions arrive at any time.
const KEYS_RETRY: Duration = Duration::from_secs(5);

/// What the flow reports to. Runs on the flow's threads.
pub type OnEvent = Arc<dyn Fn(Event) + Send + Sync>;

/// Starts the flow on the real machine. `on_event` runs on the flow's threads.
pub fn start(settings: Settings, on_event: OnEvent) -> Dictation {
    let data = data_dir().unwrap_or_else(|| PathBuf::from("wispr-free"));
    let default_models = ModelStore::default_root().unwrap_or_else(|| data.join("models"));
    let history = History::default_path().unwrap_or_else(|| data.join("history.sqlite3"));
    let meetings = data.join("meetings");
    let executable = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("wispr-free"));
    let engine_host = engine_host_beside(&executable);
    // `cargo run` builds only the app.
    if !engine_host.exists() {
        eprintln!(
            "no engine host at {}: run `cargo build -p wf-engine-host`",
            engine_host.display()
        );
    }
    Dictation::start(Parts::system(
        settings,
        Arc::new(Catalog::bundled()),
        default_models,
        history,
        meetings,
        engine_host,
        on_event,
    ))
}

/// The real machine, with the flow's files at `paths` and nowhere else.
pub fn parts_at(paths: &Paths, settings: Settings, on_event: OnEvent) -> Parts {
    Parts::system(
        settings,
        Arc::new(Catalog::bundled()),
        paths.models.clone(),
        paths.history.clone(),
        paths.meetings.clone(),
        paths.engine_host.clone(),
        on_event,
    )
}

/// Listens to the keyboard for as long as the returned value lives. While the OS refuses, the pill
/// says why and this keeps trying. The way a bare binary runs: in the app bundle, onboarding's
/// watch does all this through [`KeyWatch::refresh`].
pub fn listen_to_keys(dictation: &Dictation, clock: &dyn Clock) -> KeyWatch {
    let watch = KeyWatch::idle(dictation, clock);
    if !watch.try_listen_and_report() {
        let this = watch.clone();
        watch.retry.every(
            KEYS_RETRY,
            Box::new(move || {
                if this.try_listen_and_report() {
                    this.retry.stop();
                }
            }),
        );
    }
    watch
}

#[derive(Clone)]
pub struct KeyWatch {
    dictation: Dictation,
    listener: Rc<RefCell<Option<KeyListener>>>,
    retry: Rc<dyn Timer>,
    permissions: Rc<Cell<Option<Permissions>>>,
}

impl KeyWatch {
    /// Not listening yet.
    pub fn idle(dictation: &Dictation, clock: &dyn Clock) -> Self {
        Self {
            dictation: dictation.clone(),
            listener: Rc::default(),
            retry: Rc::from(clock.timer()),
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

/// Puts `id` at the top of the microphone ranking, keeping the rest in order.
pub fn rank_first(ranking: &mut Vec<String>, id: String) {
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
