//! What the Settings pages edit, mapped to and from the persisted settings, including the rules
//! that tie the quality tier to its stages and models (spec §5, §6).

use wf_core::settings::{
    CleanupStrength, HistoryRetention, KeepModels, MediaWhileDictating, PipelineTier, Settings,
    Style, Theme,
};
use wf_models::{Catalog, Tier};
use wispr_free::SettingsValues;
use wispr_free::settings_window::values::{ModelChoices, apply_values, values_of};

fn fixtures() -> (Catalog, ModelChoices) {
    let catalog = Catalog::bundled();
    let choices = ModelChoices::from_catalog(&catalog);
    (catalog, choices)
}

fn applied(settings: &Settings, edit: impl FnOnce(&mut SettingsValues)) -> Settings {
    let (catalog, choices) = fixtures();
    let mut values = values_of(settings, &catalog, &choices);
    edit(&mut values);
    let mut result = settings.clone();
    apply_values(&mut result, &values, &catalog, &choices);
    result
}

#[test]
fn showing_the_values_and_saving_them_changes_nothing() {
    let settings = Settings::default();
    assert_eq!(applied(&settings, |_| {}), settings);
}

#[test]
fn the_default_stages_are_those_of_the_default_tier() {
    let (catalog, _) = fixtures();
    let pipeline = Settings::default().pipeline;
    let preset = catalog.preset(Tier::Balanced);

    assert_eq!(pipeline.tier, PipelineTier::Balanced);
    assert_eq!(pipeline.denoise, preset.denoise);
    assert_eq!(pipeline.vad, preset.vad);
    assert_eq!(pipeline.llm_cleanup, preset.llm_cleanup);
}

#[test]
fn every_edited_setting_reaches_the_settings() {
    let (catalog, _) = fixtures();
    let asr = "whisper-small-q8_0";
    assert!(catalog.model(asr).is_some());

    let mut expected = Settings::default();
    expected.general.app_language = "en".into();
    expected.general.theme = Theme::Dark;
    expected.general.launch_at_login = true;
    expected.general.show_pill = false;
    expected.general.show_in_dock = true;
    expected.pipeline.tier = PipelineTier::Custom;
    expected.pipeline.denoise = true;
    expected.pipeline.vad = false;
    expected.pipeline.llm_cleanup = false;
    expected.pipeline.cleanup_strength = CleanupStrength::High;
    expected.pipeline.vad_auto_stop = true;
    expected.pipeline.vad_auto_stop_seconds = 9;
    expected.pipeline.asr_model = Some(asr.into());
    expected.pipeline.vad_model = catalog.preset(Tier::Lite).vad_model.clone();
    expected.pipeline.llm_model = catalog.preset(Tier::High).llm_model.clone();
    expected.pipeline.keep_models_loaded = KeepModels::UnloadAfterUse;
    expected.style.personal_messages = Style::VeryCasual;
    expected.style.work_messages = Style::Excited;
    expected.style.email = Style::Casual;
    expected.style.other = Style::VeryCasual;
    expected.style.auto_add_corrections = false;
    expected.meetings.auto_detect = true;
    expected.meetings.live_transcript = false;
    expected.meetings.refine_pass = false;
    expected.meetings.diarization = false;
    expected.meetings.keep_audio = false;
    expected.meetings.max_length_minutes = 90;
    expected.meetings.summary_template = "Decisions first.".into();
    expected.media.sounds = false;
    expected.media.while_dictating = MediaWhileDictating::Mute;
    expected.privacy.context_awareness = false;
    expected.privacy.history_retention = HistoryRetention::Delete24h;

    assert_eq!(applied(&expected, |_| {}), expected);

    let (catalog, choices) = fixtures();
    let values = values_of(&expected, &catalog, &choices);
    let mut from_defaults = Settings::default();
    from_defaults.pipeline.tier = PipelineTier::Custom;
    apply_values(&mut from_defaults, &values, &catalog, &choices);
    assert_eq!(from_defaults, expected);
}

#[test]
fn choices_are_shown_in_the_order_of_the_pages() {
    let (catalog, choices) = fixtures();
    let mut settings = Settings::default();
    settings.general.theme = Theme::Dark;
    settings.pipeline.tier = PipelineTier::High;
    settings.pipeline.cleanup_strength = CleanupStrength::None;
    settings.pipeline.keep_models_loaded = KeepModels::UnloadAfterUse;
    settings.style.email = Style::Excited;
    settings.media.while_dictating = MediaWhileDictating::Pause;
    settings.privacy.history_retention = HistoryRetention::Never;

    let values = values_of(&settings, &catalog, &choices);

    assert_eq!(values.theme, 2);
    assert_eq!(values.tier, 2);
    assert_eq!(values.cleanup_strength, 0);
    assert_eq!(values.keep_models_loaded, 2);
    assert_eq!(values.style_email, 3);
    assert_eq!(values.media_while_dictating, 2);
    assert_eq!(values.history_retention, 2);
}

#[test]
fn a_preset_tier_shows_the_models_the_catalog_picks_for_it() {
    let (catalog, choices) = fixtures();
    let mut settings = Settings::default();
    settings.pipeline.tier = PipelineTier::Lite;

    let values = values_of(&settings, &catalog, &choices);

    let preset = catalog.preset(Tier::Lite);
    assert_eq!(
        choices.asr_ids()[values.asr_model as usize],
        preset.asr_model
    );
    assert_eq!(values.llm_model, -1, "Lite has no LLM");
}

#[test]
fn picking_a_tier_takes_on_its_stages_and_follows_the_catalog() {
    let (catalog, _) = fixtures();
    let mut settings = Settings::default();
    settings.pipeline.tier = PipelineTier::Custom;
    settings.pipeline.denoise = true;
    settings.pipeline.asr_model = Some("something-custom".into());

    let result = applied(&settings, |values| values.tier = 0);

    let preset = catalog.preset(Tier::Lite);
    assert_eq!(result.pipeline.tier, PipelineTier::Lite);
    assert_eq!(result.pipeline.denoise, preset.denoise);
    assert_eq!(result.pipeline.vad, preset.vad);
    assert_eq!(result.pipeline.llm_cleanup, preset.llm_cleanup);
    assert_eq!(result.pipeline.asr_model, None);
    assert_eq!(result.pipeline.vad_model, None);
    assert_eq!(result.pipeline.llm_model, None);
}

#[test]
fn changing_a_stage_makes_the_tier_custom_and_pins_its_models() {
    let (catalog, _) = fixtures();
    let settings = Settings::default();
    let preset = catalog.preset(Tier::Balanced);

    let result = applied(&settings, |values| values.denoise = !values.denoise);

    assert_eq!(result.pipeline.tier, PipelineTier::Custom);
    assert!(result.pipeline.denoise);
    assert_eq!(
        result.pipeline.asr_model.as_deref(),
        Some(preset.asr_model.as_str())
    );
    assert_eq!(result.pipeline.vad_model, preset.vad_model);
    assert_eq!(result.pipeline.llm_model, preset.llm_model);
}

#[test]
fn picking_another_model_makes_the_tier_custom() {
    let (_, choices) = fixtures();
    let settings = Settings::default();
    let other = choices
        .asr_ids()
        .iter()
        .position(|id| id == "whisper-small-q8_0")
        .unwrap() as i32;

    let result = applied(&settings, |values| values.asr_model = other);

    assert_eq!(result.pipeline.tier, PipelineTier::Custom);
    assert_eq!(
        result.pipeline.asr_model.as_deref(),
        Some("whisper-small-q8_0")
    );
}

#[test]
fn choosing_custom_keeps_what_the_tier_had_and_pins_its_models() {
    let (catalog, _) = fixtures();
    let settings = Settings::default();

    let result = applied(&settings, |values| values.tier = 3);

    assert_eq!(result.pipeline.tier, PipelineTier::Custom);
    assert_eq!(result.pipeline.denoise, settings.pipeline.denoise);
    assert_eq!(
        result.pipeline.asr_model.as_deref(),
        Some(catalog.preset(Tier::Balanced).asr_model.as_str())
    );
}

#[test]
fn a_custom_pick_the_catalog_no_longer_lists_survives_an_unrelated_edit() {
    let mut settings = Settings::default();
    settings.pipeline.tier = PipelineTier::Custom;
    settings.pipeline.asr_model = Some("removed-model".into());

    let result = applied(&settings, |values| values.sounds = false);

    assert_eq!(result.pipeline.asr_model.as_deref(), Some("removed-model"));
    assert!(!result.media.sounds);
}

#[test]
fn an_app_language_this_build_does_not_offer_survives_an_unrelated_edit() {
    let mut settings = Settings::default();
    settings.general.app_language = "it".into();

    let result = applied(&settings, |values| values.sounds = false);

    assert_eq!(result.general.app_language, "it");
}
