//! The settings model and its file: defaults from the spec, and every setting surviving a restart.

use std::fs;

use wf_core::settings::{
    CleanupStrength, HistoryRetention, KeepModels, MediaWhileDictating, PipelineTier, Replacement,
    Settings, SettingsStore, Snippet, Style, Theme,
};
use wf_core::shortcut::{Combo, Key, Side};

/// A value of every setting that differs from its default.
fn every_setting_changed() -> Settings {
    let mut settings = Settings::default();

    let general = &mut settings.general;
    general.microphones = vec!["coreaudio:usb".into(), "coreaudio:built-in".into()];
    general.languages = vec!["it".into(), "en".into()];
    general.app_language = "en".into();
    general.theme = Theme::Dark;
    general.launch_at_login = true;
    general.show_pill = false;
    general.show_in_dock = true;

    let shortcuts = &mut settings.shortcuts;
    let combo = |c| {
        Combo::new([
            Key::Ctrl(Side::Either),
            Key::Alt(Side::Either),
            Key::Char(c),
        ])
    };
    shortcuts.push_to_talk = combo('1');
    shortcuts.hands_free = combo('2');
    shortcuts.cancel = combo('3');
    shortcuts.paste_last = combo('4');
    shortcuts.copy_last = combo('5');
    shortcuts.meeting_notes = combo('6');

    let pipeline = &mut settings.pipeline;
    pipeline.tier = PipelineTier::Custom;
    pipeline.denoise = true;
    pipeline.vad = false;
    pipeline.llm_cleanup = false;
    pipeline.cleanup_strength = CleanupStrength::High;
    pipeline.vad_auto_stop = true;
    pipeline.vad_auto_stop_seconds = 7;
    pipeline.asr_model = Some("whisper-small-q8_0".into());
    pipeline.vad_model = Some("silero-vad-v5".into());
    pipeline.llm_model = Some("qwen3-1.7b".into());
    pipeline.keep_models_loaded = KeepModels::Always;

    let style = &mut settings.style;
    style.personal_messages = Style::VeryCasual;
    style.work_messages = Style::Excited;
    style.email = Style::Casual;
    style.other = Style::Excited;
    style.dictionary = vec!["Slint".into(), "Infelici".into()];
    style.replacements = vec![Replacement {
        wrong: "wisper".into(),
        right: "Whisper".into(),
    }];
    style.snippets = vec![Snippet {
        trigger: "my address".into(),
        text: "Via Roma 1\nMilano".into(),
    }];
    style.auto_add_corrections = false;

    let meetings = &mut settings.meetings;
    meetings.auto_detect = true;
    meetings.live_transcript = false;
    meetings.refine_pass = false;
    meetings.diarization = false;
    meetings.keep_audio = false;
    meetings.max_length_minutes = 45;
    meetings.summary_template = "Decisions first.".into();

    let media = &mut settings.media;
    media.sounds = false;
    media.while_dictating = MediaWhileDictating::Pause;

    let privacy = &mut settings.privacy;
    privacy.context_awareness = false;
    privacy.history_retention = HistoryRetention::Never;

    settings
}

/// The dotted path of every leaf of a settings file.
fn leaves(value: &toml::Value, path: String, out: &mut Vec<(String, toml::Value)>) {
    if let toml::Value::Table(table) = value {
        for (key, value) in table {
            leaves(value, format!("{path}.{key}"), out);
        }
    } else {
        out.push((path, value.clone()));
    }
}

fn leaves_of(settings: &Settings) -> Vec<(String, toml::Value)> {
    let mut out = Vec::new();
    leaves(
        &toml::Value::try_from(settings).unwrap(),
        String::new(),
        &mut out,
    );
    out
}

#[test]
fn the_fixture_really_changes_every_setting() {
    let defaults = leaves_of(&Settings::default());
    let changed = leaves_of(&every_setting_changed());
    for default in &defaults {
        assert!(
            changed.iter().all(|leaf| leaf != default),
            "{} kept its default",
            default.0
        );
        assert!(
            changed.iter().any(|leaf| leaf.0 == default.0),
            "{} went missing",
            default.0
        );
    }
    // The model picks are unset by default, so a file only has them once they are changed.
    assert_eq!(changed.len(), defaults.len() + 3);
}

#[test]
fn defaults_follow_the_spec() {
    let settings = Settings::default();

    assert_eq!(settings.general.theme, Theme::System);
    assert!(settings.general.microphones.is_empty(), "auto");
    assert!(settings.general.languages.is_empty(), "auto-detect");
    assert!(settings.general.show_pill);

    assert!(!settings.pipeline.denoise);
    assert!(settings.pipeline.vad);
    assert_eq!(settings.pipeline.cleanup_strength, CleanupStrength::Light);
    assert!(!settings.pipeline.vad_auto_stop);
    assert_eq!(settings.pipeline.vad_auto_stop_seconds, 3);

    assert!(!settings.meetings.auto_detect, "never without opt-in");
    assert!(settings.media.sounds);
    assert_eq!(settings.media.while_dictating, MediaWhileDictating::Nothing);
    assert!(settings.privacy.context_awareness);
    assert_eq!(settings.privacy.history_retention, HistoryRetention::Keep);
}

#[test]
fn every_setting_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("settings.toml");
    let expected = every_setting_changed();

    let store = SettingsStore::open(&path);
    store
        .update(|settings| *settings = expected.clone())
        .unwrap();
    drop(store);

    assert_eq!(SettingsStore::open(&path).get(), expected);
}

#[test]
fn a_first_run_has_defaults_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.toml");

    let store = SettingsStore::open(&path);

    assert_eq!(store.get(), Settings::default());
    assert!(!path.exists());
}

#[test]
fn a_setting_missing_from_the_file_keeps_its_default() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.toml");
    fs::write(
        &path,
        "[general]\ntheme = \"dark\"\n[shortcuts]\npaste_last = [\"Ctrl\", \"J\"]\n",
    )
    .unwrap();

    let settings = SettingsStore::open(&path).get();

    assert_eq!(settings.general.theme, Theme::Dark);
    assert!(settings.general.show_pill);
    assert_eq!(
        settings.shortcuts.paste_last,
        Combo::new([Key::Ctrl(Side::Either), Key::Char('J')])
    );
    assert_eq!(
        settings.shortcuts.cancel,
        Settings::default().shortcuts.cancel
    );
}

#[test]
fn a_setting_this_version_does_not_know_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.toml");
    fs::write(&path, "future = 1\n[general]\nshow_pill = false\nnew = 2\n").unwrap();

    assert!(!SettingsStore::open(&path).get().general.show_pill);
}

#[test]
fn a_damaged_file_is_set_aside_and_defaults_are_used() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.toml");
    fs::write(&path, "[general\nthis is not toml").unwrap();

    let store = SettingsStore::open(&path);

    assert_eq!(store.get(), Settings::default());
    let set_aside = store.set_aside().expect("reports the damaged copy");
    assert_eq!(
        fs::read_to_string(set_aside).unwrap(),
        "[general\nthis is not toml"
    );
    assert!(!path.exists());
}

#[test]
fn a_value_of_the_wrong_kind_is_treated_as_damage_too() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.toml");
    fs::write(&path, "[general]\ntheme = \"purple\"\n").unwrap();

    let store = SettingsStore::open(&path);

    assert_eq!(store.get(), Settings::default());
    assert!(store.set_aside().is_some());
}

#[test]
fn an_update_leaves_no_temporary_file_behind() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.toml");

    let store = SettingsStore::open(&path);
    store
        .update(|settings| settings.media.sounds = false)
        .unwrap();

    let names: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, ["settings.toml"]);
}

#[test]
fn a_failed_save_reports_the_error_and_keeps_the_new_value_for_this_run() {
    let dir = tempfile::tempdir().unwrap();
    let not_a_dir = dir.path().join("file");
    fs::write(&not_a_dir, "").unwrap();

    let store = SettingsStore::open(not_a_dir.join("settings.toml"));

    assert!(
        store
            .update(|settings| settings.media.sounds = false)
            .is_err()
    );
    assert!(!store.get().media.sounds);
}

#[test]
fn the_store_can_be_shared_with_worker_threads() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SettingsStore>();
}

#[test]
fn the_default_path_is_a_toml_file_in_the_app_folder() {
    let path = SettingsStore::default_path().expect("a home folder");
    assert!(path.ends_with("settings.toml"));
    assert!(path.to_string_lossy().contains("wispr-free"));
}
