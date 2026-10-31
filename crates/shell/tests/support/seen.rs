#![allow(dead_code)]

//! What the Settings pages show, as a window would hold it.

use wf_core::settings::{Replacement, Snippet};
use wf_shell::lists::Rows;
use wf_shell::settings::rows::{LanguageRow, MicrophoneLine, ModelRow, ShortcutRow};
use wf_shell::settings::values::SettingsValues;
use wf_shell::settings::{PageFacts, Shown};

#[derive(Clone, Default)]
pub struct Seen {
    /// How many times the whole list of models was replaced.
    pub model_rebuilds: u32,
    pub facts: Option<PageFacts>,
    pub notice: String,
    pub values: SettingsValues,
    pub vad_forced: bool,
    pub shortcuts: Vec<ShortcutRow>,
    pub recording_action: i32,
    pub recording_keys: String,
    pub shortcut_error: String,
    pub microphones: Vec<MicrophoneLine>,
    pub microphone_auto: bool,
    pub microphone_testing: bool,
    pub microphone_level: f32,
    pub microphone_error: String,
    pub languages_auto: bool,
    pub languages: Vec<LanguageRow>,
    pub dictionary: Vec<String>,
    pub replacements: Vec<Replacement>,
    pub snippets: Vec<Snippet>,
    pub hardware_summary: String,
    pub recommended_tier: String,
    pub models: Vec<ModelRow>,
    pub models_in_use_summary: String,
    pub downloaded_summary: String,
    pub asr_options: Vec<String>,
    pub vad_options: Vec<String>,
    pub llm_options: Vec<String>,
    pub model_busy: bool,
    pub model_error: String,
    pub models_folder: String,
    pub models_folder_custom: bool,
    pub models_folder_missing: bool,
    pub pending_folder: String,
    pub pending_summary: String,
    pub moving_models: bool,
    pub move_progress: f32,
    pub data_status: String,
}

impl Seen {
    /// Before anything is shown, as a window starts.
    pub fn new() -> Self {
        Self {
            recording_action: -1,
            microphone_auto: true,
            languages_auto: true,
            ..Self::default()
        }
    }

    pub fn take(&mut self, shown: Shown) {
        match shown {
            Shown::Facts(facts) => self.facts = Some(facts),
            Shown::Notice(text) => self.notice = text,
            Shown::Values(values) => self.values = *values,
            Shown::VadForced(forced) => self.vad_forced = forced,
            Shown::Shortcuts(rows) => self.shortcuts = rows,
            Shown::RecordingAction(index) => self.recording_action = index,
            Shown::RecordingKeys(keys) => self.recording_keys = keys,
            Shown::ShortcutError(error) => self.shortcut_error = error,
            Shown::Microphones(lines) => self.microphones = lines,
            Shown::MicrophoneAuto(auto) => self.microphone_auto = auto,
            Shown::MicrophoneTesting(testing) => self.microphone_testing = testing,
            Shown::MicrophoneLevel(level) => self.microphone_level = level,
            Shown::MicrophoneError(error) => self.microphone_error = error,
            Shown::LanguagesAuto(auto) => self.languages_auto = auto,
            Shown::Languages(rows) => self.languages = rows,
            Shown::Dictionary(words) => self.dictionary = words,
            Shown::Replacements(change) => change.apply(&mut self.replacements),
            Shown::Snippets(change) => change.apply(&mut self.snippets),
            Shown::HardwareSummary(summary) => self.hardware_summary = summary,
            Shown::RecommendedTier(tier) => self.recommended_tier = tier,
            Shown::Models(change) => {
                self.model_rebuilds += u32::from(matches!(change, Rows::All(_)));
                change.apply(&mut self.models);
            }
            Shown::ModelsInUseSummary(summary) => self.models_in_use_summary = summary,
            Shown::DownloadedSummary(summary) => self.downloaded_summary = summary,
            Shown::ModelMenus { asr, vad, llm } => {
                self.asr_options = asr;
                self.vad_options = vad;
                self.llm_options = llm;
            }
            Shown::ModelBusy(busy) => self.model_busy = busy,
            Shown::ModelError(error) => self.model_error = error,
            Shown::ModelsFolder(folder) => self.models_folder = folder,
            Shown::ModelsFolderCustom(custom) => self.models_folder_custom = custom,
            Shown::ModelsFolderMissing(missing) => self.models_folder_missing = missing,
            Shown::PendingFolder(folder) => self.pending_folder = folder,
            Shown::PendingSummary(summary) => self.pending_summary = summary,
            Shown::MovingModels(moving) => self.moving_models = moving,
            Shown::MoveProgress(fraction) => self.move_progress = fraction,
            Shown::DataStatus(status) => self.data_status = status,
        }
    }

    /// The pages filter the list of models this way.
    pub fn downloaded_models(&self) -> Vec<ModelRow> {
        let rows = self.models.iter().filter(|row| row.installed);
        rows.cloned().collect()
    }

    pub fn available_models(&self) -> Vec<ModelRow> {
        let rows = self.models.iter().filter(|row| !row.installed);
        rows.cloned().collect()
    }

    pub fn models_in_use(&self) -> Vec<ModelRow> {
        let rows = self.models.iter().filter(|row| !row.used_for.is_empty());
        rows.cloned().collect()
    }
}
