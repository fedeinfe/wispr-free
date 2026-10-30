//! What a dictation needs from the settings and the model folder: which models, and whether they
//! are there. Checked before the mic opens, so a missing model is an error the user can act on
//! rather than a dictation that fails after they spoke.

use std::path::PathBuf;

use wf_core::settings::{PipelineTier, Settings};
use wf_models::{Catalog, Engine, ModelEntry, ModelStatus, ModelStore, Tier};

/// The tier's own stages and models, or `None` for Custom.
pub fn preset_tier(tier: PipelineTier) -> Option<Tier> {
    match tier {
        PipelineTier::Lite => Some(Tier::Lite),
        PipelineTier::Balanced => Some(Tier::Balanced),
        PipelineTier::High => Some(Tier::High),
        PipelineTier::Custom => None,
    }
}

/// The models in effect, in the order ASR, VAD, LLM: the settings' own picks, or the catalog's for
/// a preset tier.
pub fn effective_models(settings: &Settings, catalog: &Catalog) -> [Option<String>; 3] {
    let pipeline = &settings.pipeline;
    match preset_tier(pipeline.tier) {
        Some(tier) => {
            let preset = catalog.preset(tier);
            [
                Some(preset.asr_model.clone()),
                preset.vad_model.clone(),
                preset.llm_model.clone(),
            ]
        }
        None => [
            pipeline.asr_model.clone(),
            pipeline.vad_model.clone(),
            pipeline.llm_model.clone(),
        ],
    }
}

/// Whisper invents text on noise, so VAD stays on with it whatever the setting says (spec §3).
pub fn vad_forced(settings: &Settings, catalog: &Catalog) -> bool {
    let [asr, ..] = effective_models(settings, catalog);
    asr.and_then(|id| catalog.model(&id))
        .is_some_and(|model| model.engine == Engine::WhisperCpp)
}

/// Whether the VAD stage runs.
pub fn vad_on(settings: &Settings, catalog: &Catalog) -> bool {
    settings.pipeline.vad || vad_forced(settings, catalog)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AsrModel {
    /// A `ggml-*.bin` file.
    Whisper(PathBuf),
    /// The folder holding a transducer's files.
    Sherpa(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LlmModel {
    pub file: PathBuf,
    /// Read the whole file into memory instead of mapping it. A mapped file on a drive that gets
    /// unplugged faults on the next page it needs, so models in a chosen folder go in RAM.
    pub in_ram: bool,
}

/// The files one dictation runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub asr: AsrModel,
    pub vad: Option<PathBuf>,
    pub llm: Option<LlmModel>,
    /// The LLM stage is on but can't run. The dictation carries on with rule cleanup, and the pill
    /// says why once it's over.
    pub llm_problem: Option<String>,
}

/// Works out the [`Plan`]. The error is written for the pill's tooltip.
pub fn plan(settings: &Settings, catalog: &Catalog, store: &ModelStore) -> Result<Plan, String> {
    if !store.available() {
        return Err(format!(
            "Models folder not connected: {}",
            store.root().display()
        ));
    }
    let [asr, vad, llm] = effective_models(settings, catalog);
    let asr = installed(asr.as_deref(), "speech", catalog, store)?;
    let asr_dir = store.model_dir(&asr.id);
    let asr_model = match asr.engine {
        Engine::WhisperCpp => AsrModel::Whisper(asr_dir.join(&asr.files[0].name)),
        Engine::SherpaOnnx => AsrModel::Sherpa(asr_dir),
        Engine::LlamaCpp => return Err(format!("{} can't transcribe", asr.name)),
    };
    let vad = if vad_on(settings, catalog) {
        let vad = installed(vad.as_deref(), "voice detection", catalog, store)?;
        Some(store.model_dir(&vad.id).join(&vad.files[0].name))
    } else {
        None
    };
    let wants_llm = settings.pipeline.llm_cleanup
        && settings.pipeline.cleanup_strength != wf_core::settings::CleanupStrength::None;
    let (llm, llm_problem) = match llm.filter(|_| wants_llm) {
        None => (None, None),
        Some(id) => match installed(Some(&id), "cleanup", catalog, store) {
            Ok(model) => {
                let llm = LlmModel {
                    file: store.model_dir(&model.id).join(&model.files[0].name),
                    in_ram: settings.pipeline.models_folder.is_some(),
                };
                (Some(llm), None)
            }
            Err(problem) => (None, Some(problem)),
        },
    };
    Ok(Plan {
        asr: asr_model,
        vad,
        llm,
        llm_problem,
    })
}

fn installed<'a>(
    id: Option<&str>,
    stage: &str,
    catalog: &'a Catalog,
    store: &ModelStore,
) -> Result<&'a ModelEntry, String> {
    let id = id.ok_or_else(|| format!("No {stage} model chosen"))?;
    let model = catalog
        .model(id)
        .ok_or_else(|| format!("Unknown {stage} model: {id}"))?;
    match store.status(model) {
        ModelStatus::Installed => Ok(model),
        _ => Err(format!("{} isn't downloaded", model.name)),
    }
}
