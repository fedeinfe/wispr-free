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

enum Job {
    /// Load now, so the next generation doesn't pay for it.
    WarmUp,
    Generate {
        prompt: LlmPrompt,
        events: Sender<Event>,
    },
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

    /// Starts loading the model and returns at once.
    pub fn warm_up(&self) {
        let _ = self.jobs.send(Job::WarmUp);
    }
}

impl Llm for LlmHost {
    fn generate(
        &mut self,
        prompt: &LlmPrompt,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<String, PipelineError> {
        let (events, replies) = mpsc::channel();
        let job = Job::Generate {
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

fn load_once<'a, F>(
    engine: &'a mut Option<Box<dyn Llm>>,
    load: &mut F,
) -> Result<&'a mut Box<dyn Llm>, PipelineError>
where
    F: FnMut() -> Result<Box<dyn Llm>, PipelineError>,
{
    match engine {
        Some(loaded) => Ok(loaded),
        None => Ok(engine.insert(load()?)),
    }
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
        let loaded = match load_once(&mut engine, &mut load) {
            Ok(loaded) => loaded,
            Err(error) => {
                if let Job::Generate { events, .. } = job {
                    let _ = events.send(Event::Done(Err(error)));
                }
                continue;
            }
        };
        let Job::Generate { prompt, events } = job else {
            continue;
        };
        if prompt.deadline.is_some_and(|d| Instant::now() >= d) {
            let _ = events.send(Event::Done(Err(PipelineError::Timeout)));
            continue;
        }
        let mut stream = |token: &str| {
            let _ = events.send(Event::Token(token.to_owned()));
        };
        let result = loaded.generate(&prompt, &mut stream);
        // An engine that died stays dead, so the next job loads a fresh one.
        let keep_engine = !matches!(result, Err(PipelineError::Engine(_)));
        let _ = events.send(Event::Done(result));
        if !keep_engine {
            engine = None;
        }
        if keep == KeepLoaded::UnloadAfterUse {
            engine = None;
        }
    }
}
