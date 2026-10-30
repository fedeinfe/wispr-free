//! The seam to the inference engines, and the cache of what is loaded.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use wf_pipeline::llm_host::{KeepLoaded, LlmHost};
use wf_pipeline::{Asr, Diarizer, Llm, LlmPrompt, PipelineError, Vad};

use crate::coordinator::Input;
use crate::plan::{AsrModel, DiarizerModel, LlmModel, Plan};

/// Loads engines. Every engine is `Send` and handles one call at a time.
pub trait Engines: Send + Sync {
    fn vad(&self, model: &Path) -> Result<Box<dyn Vad>, PipelineError>;
    fn asr(&self, model: &AsrModel) -> Result<Box<dyn Asr>, PipelineError>;
    fn llm(&self, model: &LlmModel) -> Result<Box<dyn Llm>, PipelineError>;
    fn diarizer(&self, model: &DiarizerModel) -> Result<Box<dyn Diarizer>, PipelineError>;
    /// Stops calls in flight on the engines from this scope. They return an error, and those
    /// engines must be dropped.
    fn interrupt(&self);
    /// The same engines with an interrupt of their own, so that cancelling one job can't reach
    /// another's.
    fn scope(&self) -> Arc<dyn Engines>;
}

/// The VAD and ASR engines between dictations. Dropping one unloads it.
#[derive(Default)]
pub(crate) struct Loaded {
    vad: Option<(PathBuf, Box<dyn Vad>)>,
    asr: Option<(AsrModel, Box<dyn Asr>)>,
}

impl Loaded {
    pub(crate) fn is_empty(&self) -> bool {
        self.vad.is_none() && self.asr.is_none()
    }

    /// Hands out the VAD and ASR engines for a recording, after unloading any the plan no longer
    /// names.
    pub(crate) fn prepare(
        &mut self,
        plan: &Plan,
        engines: &Arc<dyn Engines>,
    ) -> Result<Ready, PipelineError> {
        let vad = match (&plan.vad, self.vad.take()) {
            (None, _) => None,
            (Some(wanted), Some((model, vad))) if model == *wanted => Some(vad),
            (Some(wanted), _) => Some(engines.vad(wanted)?),
        };
        let asr = match self.asr.take() {
            Some((model, asr)) if model == plan.asr => asr,
            _ => engines.asr(&plan.asr)?,
        };
        Ok(Ready { vad, asr })
    }

    /// Keeps engines that came through a recording in working order.
    pub(crate) fn put_back(&mut self, plan: &Plan, vad: Option<Box<dyn Vad>>, asr: Box<dyn Asr>) {
        self.vad = plan.vad.clone().zip(vad);
        self.asr = Some((plan.asr.clone(), asr));
    }
}

/// Engines checked out for one recording.
pub(crate) struct Ready {
    pub(crate) vad: Option<Box<dyn Vad>>,
    pub(crate) asr: Box<dyn Asr>,
}

/// The one loaded LLM, shared by LLM cleanup, summaries and meeting chat. Generations run one at a
/// time in the order they came, so a dictation waits behind at most one call: a summary's are
/// short, and behind a chat answer its cleanup times out and it inserts the rule-cleaned text.
/// The other way round, two loaded copies, would cost 1.2 GB to save a little polish.
pub(crate) struct LlmSlot {
    /// A scope of its own: stopping the model must not reach the speech engines.
    engines: Arc<dyn Engines>,
    host: Mutex<Option<(LlmModel, LlmHost)>>,
    /// Meeting work using the model, which keeps it loaded whatever the timers say.
    leases: AtomicUsize,
    inbox: Sender<Input>,
}

impl LlmSlot {
    pub(crate) fn new(engines: &Arc<dyn Engines>, inbox: Sender<Input>) -> Self {
        Self {
            engines: engines.scope(),
            host: Mutex::default(),
            leases: AtomicUsize::new(0),
            inbox,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Option<(LlmModel, LlmHost)>> {
        self.host.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn host_for(&self, model: &LlmModel) -> LlmHost {
        let mut slot = self.lock();
        match &*slot {
            Some((loaded, host)) if loaded == model => host.clone(),
            _ => {
                let host = llm_host(model, &self.engines);
                *slot = Some((model.clone(), host.clone()));
                host
            }
        }
    }

    /// Readies the model a dictation names and starts it loading, so it's there when the
    /// recording ends. Naming none unloads it, unless meeting work is using it.
    pub(crate) fn prepare(&self, wanted: Option<&LlmModel>) {
        match wanted {
            Some(model) => self.host_for(model).warm_up(),
            None if !self.is_leased() => *self.lock() = None,
            None => {}
        }
    }

    pub(crate) fn host(&self) -> Option<LlmHost> {
        self.lock().as_ref().map(|(_, host)| host.clone())
    }

    /// The model for meeting work. It stays loaded for as long as the lease lives, and `cancel`
    /// stops a call of this lease, and only of this lease.
    pub(crate) fn lease(self: &Arc<Self>, model: &LlmModel, cancel: Arc<AtomicBool>) -> LeasedLlm {
        self.leases.fetch_add(1, Ordering::SeqCst);
        LeasedLlm {
            host: self.host_for(model),
            slot: Arc::clone(self),
            cancel,
        }
    }

    pub(crate) fn is_leased(&self) -> bool {
        self.leases.load(Ordering::SeqCst) > 0
    }

    pub(crate) fn is_loaded(&self) -> bool {
        self.lock().is_some()
    }

    /// Lets go of the model. It's gone once the last handle to it is.
    pub(crate) fn clear(&self) -> Option<LlmHost> {
        self.lock().take().map(|(_, host)| host)
    }

    pub(crate) fn interrupt(&self) {
        self.engines.interrupt();
    }
}

/// The LLM runs behind a host so a slow load can't hold up a dictation past its timeout. The host
/// never unloads by itself: the coordinator clears the slot by the same policy as the other
/// engines.
fn llm_host(model: &LlmModel, engines: &Arc<dyn Engines>) -> LlmHost {
    let (loader, stopper) = (Arc::clone(engines), Arc::clone(engines));
    let model = model.clone();
    LlmHost::spawn_stoppable(
        move || loader.llm(&model),
        KeepLoaded::Always,
        move || stopper.interrupt(),
    )
}

/// The shared model, on loan to meeting work.
pub(crate) struct LeasedLlm {
    host: LlmHost,
    slot: Arc<LlmSlot>,
    cancel: Arc<AtomicBool>,
}

impl Llm for LeasedLlm {
    fn generate(
        &mut self,
        prompt: &LlmPrompt,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<String, PipelineError> {
        self.host.generate_until(prompt, &self.cancel, on_token)
    }
}

impl Drop for LeasedLlm {
    fn drop(&mut self) {
        self.slot.leases.fetch_sub(1, Ordering::SeqCst);
        let _ = self.slot.inbox.send(Input::LlmReturned);
    }
}
