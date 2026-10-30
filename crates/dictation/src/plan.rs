//! What a dictation needs from the settings and the model folder: which models, and whether they
//! are there. Checked before the mic opens, so a missing model is an error the user can act on
//! rather than a dictation that fails after they spoke.

use std::path::PathBuf;

use wf_core::settings::{PipelineTier, Settings};
use wf_meetings::MeetingPlan;
use wf_models::{Catalog, Engine, ModelEntry, ModelKind, ModelStatus, ModelStore, Tier};

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

/// Diarization's two files.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiarizerModel {
    pub segmentation: PathBuf,
    pub embedding: PathBuf,
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
    check_folder(store)?;
    let [asr, vad, llm] = effective_models(settings, catalog);
    let asr_model = asr_model(asr.as_deref(), catalog, store)?;
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
            Ok(model) => (Some(llm_model(model, settings, store)), None),
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

fn asr_model(id: Option<&str>, catalog: &Catalog, store: &ModelStore) -> Result<AsrModel, String> {
    let asr = installed(id, "speech", catalog, store)?;
    let dir = store.model_dir(&asr.id);
    match asr.engine {
        Engine::WhisperCpp => Ok(AsrModel::Whisper(dir.join(&asr.files[0].name))),
        Engine::SherpaOnnx => Ok(AsrModel::Sherpa(dir)),
        Engine::LlamaCpp => Err(format!("{} can't transcribe", asr.name)),
    }
}

fn llm_model(model: &ModelEntry, settings: &Settings, store: &ModelStore) -> LlmModel {
    LlmModel {
        file: store.model_dir(&model.id).join(&model.files[0].name),
        in_ram: settings.pipeline.models_folder.is_some(),
    }
}

/// The language to hold ASR and the summary to: the one in "My languages", if it names just one.
pub fn single_language(settings: &Settings) -> Option<String> {
    match settings.general.languages.as_slice() {
        [only] => Some(only.clone()),
        _ => None,
    }
}

/// Diarization's two models, or none when the tier doesn't diarize. A Custom tier has no choice of
/// them, so it takes the catalog's first.
pub fn speaker_model_ids(settings: &Settings, catalog: &Catalog) -> Vec<String> {
    match preset_tier(settings.pipeline.tier) {
        Some(tier) => catalog
            .speaker_models_for_tier(tier)
            .into_iter()
            .map(|model| model.id.clone())
            .collect(),
        None => [ModelKind::Segmentation, ModelKind::Embedding]
            .into_iter()
            .filter_map(|kind| catalog.first_of_kind(kind))
            .map(|model| model.id.clone())
            .collect(),
    }
}

/// Which stages run after a meeting: what the tier can do and the user wants. The summary and the
/// chat need an LLM, but not LLM cleanup, so turning cleanup off leaves them on.
pub fn meeting_plan(settings: &Settings, catalog: &Catalog) -> MeetingPlan {
    let meetings = &settings.meetings;
    let preset = preset_tier(settings.pipeline.tier).map(|tier| catalog.preset(tier));
    let [.., llm] = effective_models(settings, catalog);
    MeetingPlan {
        refine: meetings.refine_pass && preset.is_none_or(|preset| preset.meeting_refine),
        diarize: meetings.diarization && !speaker_model_ids(settings, catalog).is_empty(),
        summarize: llm.is_some(),
        keep_audio: meetings.keep_audio,
    }
}

/// What a meeting transcribes on. It always needs VAD, whatever the dictation pipeline does.
pub fn meeting_speech(
    settings: &Settings,
    catalog: &Catalog,
    store: &ModelStore,
) -> Result<(AsrModel, PathBuf), String> {
    check_folder(store)?;
    let [asr, vad, _] = effective_models(settings, catalog);
    let asr = asr_model(asr.as_deref(), catalog, store)?;
    let vad = vad.or_else(|| Some(catalog.first_of_kind(ModelKind::Vad)?.id.clone()));
    let vad = installed(vad.as_deref(), "voice detection", catalog, store)?;
    Ok((asr, store.model_dir(&vad.id).join(&vad.files[0].name)))
}

/// What the summary and the chat run on.
pub fn meeting_llm(
    settings: &Settings,
    catalog: &Catalog,
    store: &ModelStore,
) -> Result<LlmModel, String> {
    check_folder(store)?;
    let [.., llm] = effective_models(settings, catalog);
    let model = installed(llm.as_deref(), "summary", catalog, store)?;
    Ok(llm_model(model, settings, store))
}

/// What tells the voices apart.
pub fn meeting_diarizer(
    settings: &Settings,
    catalog: &Catalog,
    store: &ModelStore,
) -> Result<DiarizerModel, String> {
    check_folder(store)?;
    let ids = speaker_model_ids(settings, catalog);
    let [segmentation, embedding] = ids.as_slice() else {
        return Err("No speaker models chosen".to_owned());
    };
    let file = |id: &str| -> Result<PathBuf, String> {
        let model = installed(Some(id), "speaker", catalog, store)?;
        Ok(store.model_dir(&model.id).join(&model.files[0].name))
    };
    Ok(DiarizerModel {
        segmentation: file(segmentation)?,
        embedding: file(embedding)?,
    })
}

fn check_folder(store: &ModelStore) -> Result<(), String> {
    if store.available() {
        Ok(())
    } else {
        Err(format!(
            "Models folder not connected: {}",
            store.root().display()
        ))
    }
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
