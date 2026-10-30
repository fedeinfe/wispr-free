//! The seam to the inference engines, and the cache of what is loaded.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use wf_pipeline::llm_host::{KeepLoaded, LlmHost};
use wf_pipeline::{Asr, Llm, PipelineError, Vad};

use crate::plan::{AsrModel, LlmModel, Plan};

/// Loads engines. Every engine is `Send` and handles one call at a time.
pub trait Engines: Send + Sync {
    fn vad(&self, model: &Path) -> Result<Box<dyn Vad>, PipelineError>;
    fn asr(&self, model: &AsrModel) -> Result<Box<dyn Asr>, PipelineError>;
    fn llm(&self, model: &LlmModel) -> Result<Box<dyn Llm>, PipelineError>;
    /// Stops calls in flight. They return an error, and those engines must be dropped.
    fn interrupt(&self);
}

/// The engines between dictations. Dropping one unloads it.
#[derive(Default)]
pub(crate) struct Loaded {
    vad: Option<(PathBuf, Box<dyn Vad>)>,
    asr: Option<(AsrModel, Box<dyn Asr>)>,
    llm: Option<(LlmModel, LlmHost)>,
}

impl Loaded {
    pub(crate) fn is_empty(&self) -> bool {
        self.vad.is_none() && self.asr.is_none() && self.llm.is_none()
    }

    /// Hands out the VAD and ASR engines for a recording, after unloading any the plan no longer
    /// names, and starts the LLM loading so it's ready when the recording ends.
    pub(crate) fn prepare(
        &mut self,
        plan: &Plan,
        engines: &Arc<dyn Engines>,
    ) -> Result<Ready, PipelineError> {
        if self.llm.as_ref().map(|(model, _)| model) != plan.llm.as_ref() {
            self.llm = plan.llm.clone().map(|model| {
                let host = llm_host(&model, engines);
                (model, host)
            });
        }
        if let Some((_, host)) = &self.llm {
            host.warm_up();
        }
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

    pub(crate) fn llm(&self) -> Option<LlmHost> {
        self.llm.as_ref().map(|(_, host)| host.clone())
    }
}

/// The LLM runs behind a host so a slow load can't hold up a dictation past its timeout. The host
/// never unloads by itself: [`Loaded`] is dropped by the same policy as the other engines.
fn llm_host(model: &LlmModel, engines: &Arc<dyn Engines>) -> LlmHost {
    let engines = Arc::clone(engines);
    let model = model.clone();
    LlmHost::spawn(move || engines.llm(&model), KeepLoaded::Always)
}

/// Engines checked out for one recording.
pub(crate) struct Ready {
    pub(crate) vad: Option<Box<dyn Vad>>,
    pub(crate) asr: Box<dyn Asr>,
}
