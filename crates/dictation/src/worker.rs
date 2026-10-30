//! The thread that touches the clipboard and the history, so that insertions never overlap and
//! paste last works while a recording runs.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant, SystemTime};

use wf_core::settings::{HistoryRetention, Settings};
use wf_core::shortcut::Platform;
use wf_history::{Dictation, History, Timings};
use wf_platform::insert::{CopyReason, InsertError, InsertOutcome};

use crate::coordinator::Input;
use crate::{Event, Insertion, Report};

/// Entries in the pill's Recent transcripts menu.
const RECENT: usize = 10;

const NOTHING_YET: &str = "Nothing to paste yet";

pub(crate) enum Job {
    Insert(Box<Insert>),
    /// With the text of the paste hint, for when no field has focus.
    PasteLast(String),
    PasteRecent(usize),
    CopyLast,
    SetRetention(HistoryRetention),
    Prune,
    Forget(Sender<()>),
}

/// A finished dictation on its way to the focused app.
pub(crate) struct Insert {
    pub(crate) id: u64,
    pub(crate) text: String,
    pub(crate) press_enter: bool,
    pub(crate) raw_text: String,
    pub(crate) app: Option<String>,
    pub(crate) duration: Duration,
    pub(crate) timings: Timings,
    pub(crate) released: Instant,
    pub(crate) cleanup_fallback: Option<String>,
    pub(crate) problem: Option<String>,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) paste_hint: String,
}

/// The toast for text that stayed on the clipboard.
pub(crate) fn paste_hint(settings: &Settings) -> String {
    let combo = settings.shortcuts.paste_last.label(Platform::current());
    format!("Copied — paste with {combo}")
}

struct Worker {
    insertion: Box<dyn Insertion>,
    history: History,
    /// The last list shown, which is what an index from the menu refers to.
    recent: Vec<String>,
    finished: Sender<Input>,
    on_event: Arc<dyn Fn(Event) + Send + Sync>,
}

pub(crate) fn run(
    jobs: Receiver<Job>,
    insertion: Box<dyn Insertion>,
    history: PathBuf,
    finished: Sender<Input>,
    on_event: Arc<dyn Fn(Event) + Send + Sync>,
) {
    let mut worker = Worker {
        insertion,
        history: History::new(history),
        recent: Vec::new(),
        finished,
        on_event,
    };
    for job in jobs {
        worker.handle(job);
    }
}

impl Worker {
    fn handle(&mut self, job: Job) {
        match job {
            Job::Insert(insert) => self.insert(*insert),
            Job::PasteLast(hint) => match self.history.last_text(SystemTime::now()) {
                Ok(Some(text)) => self.paste(&text, &hint),
                Ok(None) => self.toast(NOTHING_YET),
                Err(error) => self.toast(&format!("Couldn't read history: {error}")),
            },
            Job::PasteRecent(index) => {
                if let Some(text) = self.recent.get(index).cloned() {
                    let hint = "Copied".to_owned();
                    self.paste(&text, &hint);
                }
            }
            Job::CopyLast => match self.history.last_text(SystemTime::now()) {
                Ok(Some(text)) => {
                    if let Err(error) = self.insertion.copy(&text) {
                        self.toast(&format!("Couldn't copy: {error}"));
                    }
                }
                Ok(None) => self.toast(NOTHING_YET),
                Err(error) => self.toast(&format!("Couldn't read history: {error}")),
            },
            Job::SetRetention(retention) => {
                if let Err(error) = self.history.set_retention(retention, SystemTime::now()) {
                    self.toast(&format!("Couldn't open history: {error}"));
                }
                self.publish_recent();
            }
            Job::Prune => {
                if let Err(error) = self.history.prune(SystemTime::now()) {
                    self.toast(&format!("Couldn't clean up history: {error}"));
                }
                self.publish_recent();
            }
            Job::Forget(done) => {
                if let Err(error) = self.history.clear() {
                    self.toast(&format!("Couldn't delete history: {error}"));
                }
                // Reading the list back would open the file again.
                self.recent.clear();
                (self.on_event)(Event::Recent(Vec::new()));
                let _ = done.send(());
            }
        }
    }

    fn insert(&mut self, insert: Insert) {
        let id = insert.id;
        if insert.cancel.load(Ordering::SeqCst) {
            self.finish(id, None);
            return;
        }
        let started = Instant::now();
        let mut pasted_at = None;
        let mut reported = false;
        let finished = self.finished.clone();
        let outcome = if insert.text.is_empty() {
            None
        } else {
            let mut on_pasted = || {
                pasted_at = Some(insert.released.elapsed());
                reported = true;
                let _ = finished.send(Input::Finished { id, problem: None });
            };
            Some(self.insertion.insert(&insert.text, &mut on_pasted))
        };
        let release_to_insert = pasted_at.unwrap_or_else(|| insert.released.elapsed());

        let mut problem = insert.problem.clone();
        match &outcome {
            None | Some(Ok(InsertOutcome::Pasted { .. })) => {
                if insert.press_enter
                    && let Err(error) = self.insertion.press_enter()
                {
                    problem = Some(enter_problem(&error));
                }
            }
            Some(Ok(InsertOutcome::Copied { reason })) => match reason {
                CopyReason::NoFocusedField => self.toast(&insert.paste_hint),
                CopyReason::PasteFailed(error) => {
                    self.toast(&insert.paste_hint);
                    problem = Some(format!("Couldn't paste: {error}"));
                }
            },
            Some(Err(error)) => problem = Some(format!("Couldn't insert: {error}")),
        }

        // Sent Enter usually empties the field, so there's nothing left to fix.
        if !insert.press_enter && matches!(outcome, Some(Ok(InsertOutcome::Pasted { .. }))) {
            let _ = self.finished.send(Input::Inserted {
                id,
                text: insert.text.clone(),
                app: insert.app.clone(),
            });
        }
        let timings = Timings {
            insertion: started.elapsed(),
            ..insert.timings
        };
        let entry = Dictation {
            at: SystemTime::now(),
            raw_text: insert.raw_text,
            final_text: insert.text,
            app: insert.app,
            duration: insert.duration,
            timings,
        };
        // A failed insert keeps its text too: paste last is how the user gets it back.
        if let Err(error) = self.history.record(&entry) {
            self.toast(&format!("Couldn't save history: {error}"));
        }
        self.publish_recent();
        (self.on_event)(Event::Inserted(Report {
            release_to_insert,
            timings,
            cleanup_fallback: insert.cleanup_fallback,
        }));
        if !reported || problem.is_some() {
            self.finish(id, problem);
        }
    }

    fn paste(&mut self, text: &str, hint: &str) {
        match self.insertion.insert(text, &mut || {}) {
            Ok(InsertOutcome::Pasted { .. }) => {}
            Ok(InsertOutcome::Copied { .. }) => self.toast(hint),
            Err(error) => self.toast(&format!("Couldn't paste: {error}")),
        }
    }

    fn finish(&self, id: u64, problem: Option<String>) {
        let _ = self.finished.send(Input::Finished { id, problem });
    }

    fn toast(&self, text: &str) {
        (self.on_event)(Event::Toast(text.to_owned()));
    }

    fn publish_recent(&mut self) {
        let recent = self.history.recent(RECENT, SystemTime::now());
        self.recent = recent
            .unwrap_or_default()
            .into_iter()
            .map(|dictation| dictation.final_text)
            .filter(|text| !text.is_empty())
            .collect();
        (self.on_event)(Event::Recent(self.recent.clone()));
    }
}

fn enter_problem(error: &InsertError) -> String {
    format!("Couldn't press Enter: {error}")
}
