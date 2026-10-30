//! The model catalog: a versioned TOML manifest bundled with the app (`catalog.toml`).

use std::collections::HashSet;

use serde::Deserialize;

use crate::Tier;

const BUNDLED: &str = include_str!("../catalog.toml");

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    /// Format version; this build reads [`Catalog::SCHEMA`].
    pub schema: u32,
    /// Content version, bumped on every edit (date-based).
    pub version: String,
    tiers: Tiers,
    pub models: Vec<ModelEntry>,
    /// Diarization's models. A list of their own: [`ModelKind`] is matched exhaustively in
    /// Settings, so they join [`Self::models`] once that page can label them.
    #[serde(default)]
    pub speaker_models: Vec<SpeakerModelEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Tiers {
    lite: TierPreset,
    balanced: TierPreset,
    high: TierPreset,
}

/// A tier's stage toggles plus the model each enabled stage uses.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TierPreset {
    pub denoise: bool,
    pub vad: bool,
    pub llm_cleanup: bool,
    pub meeting_refine: bool,
    pub diarization: bool,
    pub vad_model: Option<String>,
    pub asr_model: String,
    /// Also used for meeting summaries and chat.
    pub llm_model: Option<String>,
    /// Finds who speaks when. Set exactly when `diarization` is on, like `speaker_model`.
    pub segmentation_model: Option<String>,
    /// Tells the voices apart.
    pub speaker_model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
    Vad,
    Asr,
    Llm,
}

/// The two models diarization needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpeakerModelKind {
    Segmentation,
    Embedding,
}

/// The runtime that loads the model's files (ADR-0003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
pub enum Engine {
    #[serde(rename = "sherpa-onnx")]
    SherpaOnnx,
    #[serde(rename = "whisper.cpp")]
    WhisperCpp,
    #[serde(rename = "llama.cpp")]
    LlamaCpp,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelEntry<K = ModelKind> {
    /// Stable key, also the model's folder name in the store.
    pub id: String,
    pub name: String,
    pub kind: K,
    pub engine: Engine,
    /// SPDX-style identifier shown on the Models page.
    pub license: String,
    /// ISO 639-1 codes; `"*"` means multilingual (auto-detect).
    pub languages: Vec<String>,
    /// Estimated resident memory while loaded.
    pub ram_mb: u64,
    /// Not yet validated by the benchmark (ticket 14).
    #[serde(default)]
    pub candidate: bool,
    #[serde(default)]
    pub notes: Option<String>,
    pub files: Vec<ModelFile>,
}

pub type SpeakerModelEntry = ModelEntry<SpeakerModelKind>;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFile {
    /// File name inside the model's folder.
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Lowercase hex.
    pub sha256: String,
}

impl<K> ModelEntry<K> {
    pub fn download_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

impl TierPreset {
    /// Every model the preset names, in pipeline order (VAD, ASR, LLM).
    pub fn model_ids(&self) -> impl Iterator<Item = &str> {
        self.vad_model
            .iter()
            .chain(std::iter::once(&self.asr_model))
            .chain(self.llm_model.iter())
            .map(String::as_str)
    }
}

impl Catalog {
    pub const SCHEMA: u32 = 1;

    /// The manifest compiled into this build.
    pub fn bundled() -> Catalog {
        Catalog::from_toml(BUNDLED).expect("bundled catalog.toml is valid")
    }

    /// Parses and validates a manifest, e.g. a newer one than the bundled copy.
    pub fn from_toml(text: &str) -> Result<Catalog, CatalogError> {
        let catalog: Catalog = toml::from_str(text).map_err(|e| CatalogError(e.to_string()))?;
        catalog.validate()?;
        Ok(catalog)
    }

    fn validate(&self) -> Result<(), CatalogError> {
        if self.schema != Self::SCHEMA {
            return invalid(format!(
                "schema {} (this build reads {})",
                self.schema,
                Self::SCHEMA
            ));
        }
        let mut ids = HashSet::new();
        for model in &self.models {
            validate_entry(model, &mut ids)?;
        }
        for model in &self.speaker_models {
            validate_entry(model, &mut ids)?;
        }
        for tier in [Tier::Lite, Tier::Balanced, Tier::High] {
            self.validate_preset(tier)?;
        }
        Ok(())
    }

    fn validate_preset(&self, tier: Tier) -> Result<(), CatalogError> {
        let preset = self.preset(tier);
        if preset.vad != preset.vad_model.is_some()
            || preset.llm_cleanup != preset.llm_model.is_some()
            || preset.diarization != preset.segmentation_model.is_some()
            || preset.diarization != preset.speaker_model.is_some()
        {
            return invalid(format!(
                "{tier:?}: a stage is on without a model, or off with one"
            ));
        }
        let slots = [
            (&preset.vad_model, ModelKind::Vad),
            (&Some(preset.asr_model.clone()), ModelKind::Asr),
            (&preset.llm_model, ModelKind::Llm),
        ];
        for (id, kind) in slots {
            let Some(id) = id else { continue };
            match self.model(id) {
                None => return invalid(format!("{tier:?} names unknown model {id}")),
                Some(model) if model.kind != kind => {
                    return invalid(format!("{tier:?} uses {id} ({:?}) as {kind:?}", model.kind));
                }
                Some(_) => {}
            }
        }
        let speaker_slots = [
            (&preset.segmentation_model, SpeakerModelKind::Segmentation),
            (&preset.speaker_model, SpeakerModelKind::Embedding),
        ];
        for (id, kind) in speaker_slots {
            let Some(id) = id else { continue };
            match self.speaker_model(id) {
                Some(model) if model.kind == kind => {}
                _ => return invalid(format!("{tier:?} has no {kind:?} model {id}")),
            }
        }
        Ok(())
    }

    pub fn model(&self, id: &str) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.id == id)
    }

    pub fn speaker_model(&self, id: &str) -> Option<&SpeakerModelEntry> {
        self.speaker_models.iter().find(|m| m.id == id)
    }

    pub fn preset(&self, tier: Tier) -> &TierPreset {
        match tier {
            Tier::Lite => &self.tiers.lite,
            Tier::Balanced => &self.tiers.balanced,
            Tier::High => &self.tiers.high,
        }
    }

    /// What a tier downloads.
    pub fn models_for_tier(&self, tier: Tier) -> Vec<&ModelEntry> {
        self.preset(tier)
            .model_ids()
            .filter_map(|id| self.model(id))
            .collect()
    }

    /// What a tier downloads on top of [`Self::models_for_tier`]: segmentation, then embedding.
    pub fn speaker_models_for_tier(&self, tier: Tier) -> Vec<&SpeakerModelEntry> {
        let preset = self.preset(tier);
        [&preset.segmentation_model, &preset.speaker_model]
            .into_iter()
            .flatten()
            .filter_map(|id| self.speaker_model(id))
            .collect()
    }
}

/// A manifest that doesn't parse or doesn't hold together.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid model catalog: {0}")]
pub struct CatalogError(String);

fn invalid<T>(reason: String) -> Result<T, CatalogError> {
    Err(CatalogError(reason))
}

/// Everything about an entry that doesn't depend on its kind. `ids` spans both model lists,
/// because the id is also the folder name.
fn validate_entry<'a, K>(
    model: &'a ModelEntry<K>,
    ids: &mut HashSet<&'a str>,
) -> Result<(), CatalogError> {
    if !ids.insert(model.id.as_str()) {
        return invalid(format!("duplicate model id {}", model.id));
    }
    if !is_plain_file_name(&model.id) {
        return invalid(format!("model id {:?} is not a folder name", model.id));
    }
    if model.files.is_empty() {
        return invalid(format!("{} has no files", model.id));
    }
    let mut names = HashSet::new();
    for file in &model.files {
        let at = format!("{}/{}", model.id, file.name);
        if !is_plain_file_name(&file.name) || !names.insert(file.name.as_str()) {
            return invalid(format!("bad or duplicate file name {:?}", file.name));
        }
        if !is_sha256_hex(&file.sha256) {
            return invalid(format!("{at}: sha256 must be 64 lowercase hex digits"));
        }
        if !is_allowed_url(&file.url) {
            return invalid(format!("{at}: url must be https (ADR-0004)"));
        }
    }
    Ok(())
}

/// One path component that can't escape the model folder on any OS.
fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':'])
        && !name.chars().any(char::is_control)
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_allowed_url(url: &str) -> bool {
    let loopback = [
        "http://127.0.0.1:",
        "http://127.0.0.1/",
        "http://localhost:",
        "http://localhost/",
    ];
    url.starts_with("https://") || loopback.iter().any(|p| url.starts_with(p))
}
