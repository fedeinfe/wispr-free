//! The scalar settings as the pages show them: choices as indexes into the option lists. Also the
//! rules tying the quality tier to its stages and models.

use wf_core::settings::{
    CleanupStrength, HistoryRetention, KeepModels, MediaWhileDictating, PipelineTier, Settings,
    Style, Theme,
};
use wf_dictation::plan::{effective_models, preset_tier, vad_on};
use wf_models::{Catalog, ModelKind};

use crate::SettingsValues;

// The order of each list is the order of the options in the .slint pages.
const APP_LANGUAGES: [&str; 2] = ["system", "en"];
const THEMES: [Theme; 3] = [Theme::System, Theme::Light, Theme::Dark];
const TIERS: [PipelineTier; 4] = [
    PipelineTier::Lite,
    PipelineTier::Balanced,
    PipelineTier::High,
    PipelineTier::Custom,
];
const STRENGTHS: [CleanupStrength; 4] = [
    CleanupStrength::None,
    CleanupStrength::Light,
    CleanupStrength::Medium,
    CleanupStrength::High,
];
const KEEP_MODELS: [KeepModels; 3] = [
    KeepModels::Always,
    KeepModels::FiveMinutes,
    KeepModels::UnloadAfterUse,
];
const STYLES: [Style; 4] = [
    Style::Formal,
    Style::Casual,
    Style::VeryCasual,
    Style::Excited,
];
const MEDIA: [MediaWhileDictating; 3] = [
    MediaWhileDictating::Nothing,
    MediaWhileDictating::Mute,
    MediaWhileDictating::Pause,
];
const RETENTIONS: [HistoryRetention; 3] = [
    HistoryRetention::Keep,
    HistoryRetention::Delete24h,
    HistoryRetention::Never,
];

fn index_of<T: PartialEq>(options: &[T], value: &T) -> i32 {
    options
        .iter()
        .position(|option| option == value)
        .map_or(-1, |index| index as i32)
}

/// The option at `index`, or `current` when the page has none selected.
fn pick<T: Clone>(options: &[T], index: i32, current: &T) -> T {
    usize::try_from(index)
        .ok()
        .and_then(|index| options.get(index))
        .unwrap_or(current)
        .clone()
}

/// A language this build doesn't offer shows as System and stays stored until the user picks one.
fn app_language_index(language: &str) -> i32 {
    index_of(&APP_LANGUAGES, &language).max(0)
}

/// The models the Pipeline page offers for each stage.
#[derive(Debug, Clone)]
pub struct ModelChoices {
    asr: Vec<(String, String)>,
    vad: Vec<(String, String)>,
    llm: Vec<(String, String)>,
}

impl ModelChoices {
    pub fn from_catalog(catalog: &Catalog) -> Self {
        let of_kind = |kind| {
            catalog
                .models
                .iter()
                .filter(|model| model.kind == kind)
                .map(|model| (model.id.clone(), model.name.clone()))
                .collect()
        };
        Self {
            asr: of_kind(ModelKind::Asr),
            vad: of_kind(ModelKind::Vad),
            llm: of_kind(ModelKind::Llm),
        }
    }

    pub fn asr_ids(&self) -> Vec<String> {
        ids(&self.asr)
    }

    pub fn vad_ids(&self) -> Vec<String> {
        ids(&self.vad)
    }

    pub fn llm_ids(&self) -> Vec<String> {
        ids(&self.llm)
    }

    pub fn asr_names(&self) -> Vec<String> {
        names(&self.asr)
    }

    pub fn vad_names(&self) -> Vec<String> {
        names(&self.vad)
    }

    pub fn llm_names(&self) -> Vec<String> {
        names(&self.llm)
    }
}

fn ids(choices: &[(String, String)]) -> Vec<String> {
    choices.iter().map(|(id, _)| id.clone()).collect()
}

fn names(choices: &[(String, String)]) -> Vec<String> {
    choices.iter().map(|(_, name)| name.clone()).collect()
}

/// The models the pipeline runs with, and what for. A stage that's off needs no model.
pub fn models_in_use(settings: &Settings, catalog: &Catalog) -> Vec<(String, &'static str)> {
    let [asr, vad, llm] = effective_models(settings, catalog);
    let pipeline = &settings.pipeline;
    [
        (asr, "speech recognition"),
        (vad.filter(|_| vad_on(settings, catalog)), "voice detection"),
        (llm.filter(|_| pipeline.llm_cleanup), "cleanup"),
    ]
    .into_iter()
    .filter_map(|(model, stage)| Some((model?, stage)))
    .collect()
}

fn model_index(ids: &[String], model: Option<&String>) -> i32 {
    model.map_or(-1, |model| index_of(ids, model))
}

pub fn values_of(settings: &Settings, catalog: &Catalog, choices: &ModelChoices) -> SettingsValues {
    let [asr, vad, llm] = effective_models(settings, catalog);
    SettingsValues {
        app_language: app_language_index(&settings.general.app_language),
        theme: index_of(&THEMES, &settings.general.theme),
        launch_at_login: settings.general.launch_at_login,
        show_pill: settings.general.show_pill,
        show_in_dock: settings.general.show_in_dock,
        tier: index_of(&TIERS, &settings.pipeline.tier),
        denoise: settings.pipeline.denoise,
        vad: settings.pipeline.vad,
        llm_cleanup: settings.pipeline.llm_cleanup,
        cleanup_strength: index_of(&STRENGTHS, &settings.pipeline.cleanup_strength),
        vad_auto_stop: settings.pipeline.vad_auto_stop,
        vad_auto_stop_seconds: settings.pipeline.vad_auto_stop_seconds as i32,
        asr_model: model_index(&choices.asr_ids(), asr.as_ref()),
        vad_model: model_index(&choices.vad_ids(), vad.as_ref()),
        llm_model: model_index(&choices.llm_ids(), llm.as_ref()),
        keep_models_loaded: index_of(&KEEP_MODELS, &settings.pipeline.keep_models_loaded),
        style_personal: index_of(&STYLES, &settings.style.personal_messages),
        style_work: index_of(&STYLES, &settings.style.work_messages),
        style_email: index_of(&STYLES, &settings.style.email),
        style_other: index_of(&STYLES, &settings.style.other),
        auto_add_corrections: settings.style.auto_add_corrections,
        press_enter_command: settings.style.press_enter_command,
        meeting_auto_detect: settings.meetings.auto_detect,
        meeting_live_transcript: settings.meetings.live_transcript,
        meeting_refine_pass: settings.meetings.refine_pass,
        meeting_diarization: settings.meetings.diarization,
        meeting_keep_audio: settings.meetings.keep_audio,
        meeting_max_length_minutes: settings.meetings.max_length_minutes as i32,
        meeting_summary_template: settings.meetings.summary_template.as_str().into(),
        sounds: settings.media.sounds,
        media_while_dictating: index_of(&MEDIA, &settings.media.while_dictating),
        context_awareness: settings.privacy.context_awareness,
        history_retention: index_of(&RETENTIONS, &settings.privacy.history_retention),
    }
}

/// Stores what the pages show. The caller shows [`values_of`] the result again, since a tier
/// change also moves the stages and models.
pub fn apply_values(
    settings: &mut Settings,
    values: &SettingsValues,
    catalog: &Catalog,
    choices: &ModelChoices,
) {
    let general = &mut settings.general;
    if values.app_language != app_language_index(&general.app_language) {
        let languages = APP_LANGUAGES.map(str::to_owned);
        general.app_language = pick(&languages, values.app_language, &general.app_language);
    }
    general.theme = pick(&THEMES, values.theme, &general.theme);
    general.launch_at_login = values.launch_at_login;
    general.show_pill = values.show_pill;
    general.show_in_dock = values.show_in_dock;

    apply_pipeline(settings, values, catalog, choices);

    let style = &mut settings.style;
    style.personal_messages = pick(&STYLES, values.style_personal, &style.personal_messages);
    style.work_messages = pick(&STYLES, values.style_work, &style.work_messages);
    style.email = pick(&STYLES, values.style_email, &style.email);
    style.other = pick(&STYLES, values.style_other, &style.other);
    style.auto_add_corrections = values.auto_add_corrections;
    style.press_enter_command = values.press_enter_command;

    let meetings = &mut settings.meetings;
    meetings.auto_detect = values.meeting_auto_detect;
    meetings.live_transcript = values.meeting_live_transcript;
    meetings.refine_pass = values.meeting_refine_pass;
    meetings.diarization = values.meeting_diarization;
    meetings.keep_audio = values.meeting_keep_audio;
    meetings.max_length_minutes = values.meeting_max_length_minutes.max(1) as u32;
    meetings.summary_template = values.meeting_summary_template.to_string();

    settings.media.sounds = values.sounds;
    settings.media.while_dictating = pick(
        &MEDIA,
        values.media_while_dictating,
        &settings.media.while_dictating,
    );
    settings.privacy.context_awareness = values.context_awareness;
    settings.privacy.history_retention = pick(
        &RETENTIONS,
        values.history_retention,
        &settings.privacy.history_retention,
    );
}

fn apply_pipeline(
    settings: &mut Settings,
    values: &SettingsValues,
    catalog: &Catalog,
    choices: &ModelChoices,
) {
    let [asr, vad, llm] = effective_models(settings, catalog);
    let picked = [
        picked_model(&choices.asr_ids(), values.asr_model, asr.clone()),
        picked_model(&choices.vad_ids(), values.vad_model, vad.clone()),
        picked_model(&choices.llm_ids(), values.llm_model, llm.clone()),
    ];
    let pipeline = &mut settings.pipeline;
    pipeline.cleanup_strength = pick(
        &STRENGTHS,
        values.cleanup_strength,
        &pipeline.cleanup_strength,
    );
    pipeline.vad_auto_stop = values.vad_auto_stop;
    pipeline.vad_auto_stop_seconds = values.vad_auto_stop_seconds.max(1) as u32;
    pipeline.keep_models_loaded = pick(
        &KEEP_MODELS,
        values.keep_models_loaded,
        &pipeline.keep_models_loaded,
    );

    let tier = pick(&TIERS, values.tier, &pipeline.tier);
    if tier != pipeline.tier && preset_tier(tier).is_some() {
        choose_tier(settings, tier, catalog);
        return;
    }

    let stages_changed = pipeline.denoise != values.denoise
        || pipeline.vad != values.vad
        || pipeline.llm_cleanup != values.llm_cleanup;
    let models_changed = picked != [asr, vad, llm];
    pipeline.denoise = values.denoise;
    pipeline.vad = values.vad;
    pipeline.llm_cleanup = values.llm_cleanup;
    if tier == PipelineTier::Custom || stages_changed || models_changed {
        pipeline.tier = PipelineTier::Custom;
        [pipeline.asr_model, pipeline.vad_model, pipeline.llm_model] = picked;
    }
}

/// Moves to `tier`: a preset sets the stages and follows the catalog's models, and Custom pins the
/// models in effect so they stay when the catalog changes.
pub fn choose_tier(settings: &mut Settings, tier: PipelineTier, catalog: &Catalog) {
    let pinned = effective_models(settings, catalog);
    let pipeline = &mut settings.pipeline;
    pipeline.tier = tier;
    match preset_tier(tier).map(|tier| catalog.preset(tier)) {
        Some(preset) => {
            pipeline.denoise = preset.denoise;
            pipeline.vad = preset.vad;
            pipeline.llm_cleanup = preset.llm_cleanup;
            [pipeline.asr_model, pipeline.vad_model, pipeline.llm_model] = [None, None, None];
        }
        None => [pipeline.asr_model, pipeline.vad_model, pipeline.llm_model] = pinned,
    }
}

fn picked_model(ids: &[String], index: i32, current: Option<String>) -> Option<String> {
    usize::try_from(index)
        .ok()
        .and_then(|index| ids.get(index).cloned())
        .or(current)
}
