//! The host owns a local model on its own thread, so LLM cleanup, summaries and meeting chat
//! share one loaded copy. It loads the model on first use and drops it after an idle timeout
//! (spec §1 goal 4), and a caller always gets control back by its deadline, even when the
//! engine is stuck in a model load or a decode.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crate::{Llm, LlmPrompt, PipelineError};

/// How quickly a waiting caller notices its cancel flag.
const CANCEL_POLL: Duration = Duration::from_millis(20);

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
        /// Set by the caller who gave up. A job that hasn't started is skipped.
        abandoned: Arc<AtomicBool>,
        /// Set just before the engine gets the job, so a caller knows whose call is running.
        started: Arc<AtomicBool>,
    },
}

/// A cheap, cloneable handle. The model lives until the idle timeout or the last handle drops.
/// Generations run one at a time, in order.
#[derive(Clone)]
pub struct LlmHost {
    jobs: Sender<Job>,
    stop: Arc<dyn Fn() + Send + Sync>,
}

impl LlmHost {
    /// Starts the host's thread. `load` runs there, on demand, and again after every unload.
    pub fn spawn<F>(load: F, keep: KeepLoaded) -> Self
    where
        F: FnMut() -> Result<Box<dyn Llm>, PipelineError> + Send + 'static,
    {
        Self::spawn_stoppable(load, keep, || {})
    }

    /// Like [`spawn`](Self::spawn), for an engine that can be stopped mid-call from outside.
    /// `stop` must make the running `generate` return an error, which unloads the engine.
    pub fn spawn_stoppable<F, S>(load: F, keep: KeepLoaded, stop: S) -> Self
    where
        F: FnMut() -> Result<Box<dyn Llm>, PipelineError> + Send + 'static,
        S: Fn() + Send + Sync + 'static,
    {
        let (jobs, queue) = mpsc::channel();
        thread::spawn(move || serve(queue, load, keep));
        Self {
            jobs,
            stop: Arc::new(stop),
        }
    }

    /// Starts loading the model and returns at once.
    pub fn warm_up(&self) {
        let _ = self.jobs.send(Job::WarmUp);
    }

    fn submit(&self, prompt: &LlmPrompt) -> Result<Submitted, PipelineError> {
        let (events, replies) = mpsc::channel();
        let abandoned = Arc::<AtomicBool>::default();
        let started = Arc::<AtomicBool>::default();
        let job = Job::Generate {
            prompt: prompt.clone(),
            events,
            abandoned: Arc::clone(&abandoned),
            started: Arc::clone(&started),
        };
        self.jobs.send(job).map_err(|_| stopped())?;
        Ok(Submitted {
            replies,
            abandoned,
            started,
        })
    }

    /// Like [`Llm::generate`], until `cancel` is set. Only this call is touched: one still in the
    /// queue just goes, and one that's running stops the engine, which costs a reload. Anyone
    /// else's call, running or queued, carries on.
    pub fn generate_until(
        &self,
        prompt: &LlmPrompt,
        cancel: &AtomicBool,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<String, PipelineError> {
        let call = self.submit(prompt)?;
        loop {
            if cancel.load(Ordering::SeqCst) {
                call.abandoned.store(true, Ordering::SeqCst);
                if call.started.load(Ordering::SeqCst) {
                    (self.stop)();
                }
                return Err(PipelineError::Cancelled);
            }
            let wait = prompt.deadline.map_or(CANCEL_POLL, |deadline| {
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(CANCEL_POLL)
            });
            match call.replies.recv_timeout(wait) {
                Ok(Event::Token(token)) => on_token(&token),
                Ok(Event::Done(result)) => return result,
                Err(RecvTimeoutError::Disconnected) => return Err(stopped()),
                Err(RecvTimeoutError::Timeout) => {
                    if prompt.deadline.is_some_and(|d| Instant::now() >= d) {
                        return Err(PipelineError::Timeout);
                    }
                }
            }
        }
    }
}

/// A call on its way through the queue.
struct Submitted {
    replies: Receiver<Event>,
    abandoned: Arc<AtomicBool>,
    started: Arc<AtomicBool>,
}

impl Llm for LlmHost {
    fn generate(
        &mut self,
        prompt: &LlmPrompt,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<String, PipelineError> {
        let call = self.submit(prompt)?;
        loop {
            match next_event(&call.replies, prompt.deadline)? {
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

        if let Job::Generate {
            events,
            abandoned,
            started,
            ..
        } = &job
        {
            // In this order: whichever side looks second sees the other's flag.
            started.store(true, Ordering::SeqCst);
            if abandoned.load(Ordering::SeqCst) {
                let _ = events.send(Event::Done(Err(PipelineError::Cancelled)));
                continue;
            }
        }

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
        let Job::Generate { prompt, events, .. } = job else {
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
