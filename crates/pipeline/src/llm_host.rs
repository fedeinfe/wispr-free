//! The host owns a local model on its own thread, so LLM cleanup, summaries and meeting chat
//! share one loaded copy. It loads the model on first use and drops it after an idle timeout
//! (spec §1 goal 4), and a caller always gets control back by its deadline, even when the
//! engine is stuck in a model load or a decode.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crate::{Llm, LlmPrompt, PipelineError};

/// The "keep models loaded" setting (spec §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepLoaded {
    Always,
    For(Duration),
    UnloadAfterUse,
}

impl Default for KeepLoaded {
    fn default() -> Self {
        Self::For(Duration::from_secs(5 * 60))
    }
}

enum Event {
    Token(String),
    Done(Result<String, PipelineError>),
}

struct Job {
    prompt: LlmPrompt,
    events: Sender<Event>,
}

/// A cheap, cloneable handle. The model lives until the idle timeout or the last handle drops.
/// Generations run one at a time, in order.
#[derive(Clone)]
pub struct LlmHost {
    jobs: Sender<Job>,
}

impl LlmHost {
    /// Starts the host's thread. `load` runs there, on demand, and again after every unload.
    pub fn spawn<F>(load: F, keep: KeepLoaded) -> Self
    where
        F: FnMut() -> Result<Box<dyn Llm>, PipelineError> + Send + 'static,
    {
        let (jobs, queue) = mpsc::channel();
        thread::spawn(move || serve(queue, load, keep));
        Self { jobs }
    }
}

impl Llm for LlmHost {
    fn generate(
        &mut self,
        prompt: &LlmPrompt,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<String, PipelineError> {
        let (events, replies) = mpsc::channel();
        let job = Job {
            prompt: prompt.clone(),
            events,
        };
        self.jobs.send(job).map_err(|_| stopped())?;
        loop {
            match next_event(&replies, prompt.deadline)? {
                Event::Token(token) => on_token(&token),
                Event::Done(result) => return result,
            }
        }
    }
}

fn next_event(
    replies: &Receiver<Event>,
    deadline: Option<Instant>,
) -> Result<Event, PipelineError> {
    let Some(deadline) = deadline else {
        return replies.recv().map_err(|_| stopped());
    };
    replies
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|error| match error {
            RecvTimeoutError::Timeout => PipelineError::Timeout,
            RecvTimeoutError::Disconnected => stopped(),
        })
}

fn stopped() -> PipelineError {
    PipelineError::Engine("the LLM host thread stopped".into())
}

fn serve<F>(queue: Receiver<Job>, mut load: F, keep: KeepLoaded)
where
    F: FnMut() -> Result<Box<dyn Llm>, PipelineError>,
{
    let mut engine: Option<Box<dyn Llm>> = None;
    loop {
        let job = match (keep, &engine) {
            (KeepLoaded::For(idle), Some(_)) => match queue.recv_timeout(idle) {
                Ok(job) => job,
                Err(RecvTimeoutError::Timeout) => {
                    engine = None;
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            },
            _ => match queue.recv() {
                Ok(job) => job,
                Err(_) => return,
            },
        };

        // Keep loading past a lapsed deadline: a slow load then serves the next caller.
        if engine.is_none() {
            match load() {
                Ok(loaded) => engine = Some(loaded),
                Err(error) => {
                    let _ = job.events.send(Event::Done(Err(error)));
                    continue;
                }
            }
        }
        if job.prompt.deadline.is_some_and(|d| Instant::now() >= d) {
            let _ = job.events.send(Event::Done(Err(PipelineError::Timeout)));
            continue;
        }
        if let Some(loaded) = &mut engine {
            let events = &job.events;
            let mut stream = |token: &str| {
                let _ = events.send(Event::Token(token.to_owned()));
            };
            let result = loaded.generate(&job.prompt, &mut stream);
            let _ = job.events.send(Event::Done(result));
        }
        if keep == KeepLoaded::UnloadAfterUse {
            engine = None;
        }
    }
}
