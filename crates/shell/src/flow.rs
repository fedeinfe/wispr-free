//! What the app does around the dictation flow: starting it, and keeping the keyboard connected to
//! it. The flow itself lives in `wf_dictation`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use wf_core::settings::Settings;
use wf_dictation::{Dictation, Event, Parts};
use wf_models::Catalog;
use wf_platform::keys::{self, KeyListener};
use wf_platform::permissions::Permissions;

use crate::paths::Paths;

/// What the flow reports to. Runs on the flow's threads.
pub type OnEvent = Arc<dyn Fn(Event) + Send + Sync>;

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

#[derive(Clone)]
pub struct KeyWatch {
    dictation: Dictation,
    listener: Rc<RefCell<Option<KeyListener>>>,
    permissions: Rc<Cell<Option<Permissions>>>,
}

impl KeyWatch {
    /// Not listening yet.
    pub fn idle(dictation: &Dictation) -> Self {
        Self {
            dictation: dictation.clone(),
            listener: Rc::default(),
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
