//! What the pages show, whole, for a front end that can't hold a [`crate::settings::View`]'s
//! changes the way a Slint model does. [`Folded`] takes each [`Shown`] and gives back one record.

use wf_core::settings::{PipelineTier, Replacement, Snippet};

use super::page::SettingsPage;
use super::rows::{LanguageRow, MicrophoneLine, ModelRow, ShortcutRow, StageModel, stage_model};
use super::values::{SettingsValues, TIERS, TierRow, tier_rows};
use super::{PageFacts, Shown};

/// A stage's menu of models.
#[derive(Debug, Clone, Default, PartialEq, uniffi::Record)]
pub struct StageMenu {
    pub options: Vec<String>,
    /// A place in `options`, or -1.
    pub selected: i32,
    pub model: Option<StageModel>,
}

/// One line of the ranked microphones.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MicrophoneRow {
    pub name: String,
    /// The line itself: its place, its name and whether it's connected.
    pub title: String,
}

fn microphone_rows(lines: Vec<MicrophoneLine>) -> Vec<MicrophoneRow> {
    let rows = lines.into_iter().zip(1..).map(|(line, place)| {
        let unplugged = if line.connected {
            ""
        } else {
            " (not connected)"
        };
        MicrophoneRow {
            title: format!("{place}. {}{unplugged}", line.name),
            name: line.name,
        }
    });
    rows.collect()
}

/// Everything the pages show that doesn't come straight from the settings.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PagesState {
    pub is_macos: bool,
    pub version: String,
    pub data_folder: String,
    pub notice: String,
    /// The page that raised the notice, the only one to show it.
    pub notice_page: Option<SettingsPage>,
    pub vad_forced: bool,
    pub shortcuts: Vec<ShortcutRow>,
    pub recording_action: Option<u32>,
    pub recording_keys: String,
    pub shortcut_error: String,
    pub microphones: Vec<MicrophoneRow>,
    pub microphone_auto: bool,
    pub microphone_testing: bool,
    pub microphone_error: String,
    pub languages_auto: bool,
    pub languages: Vec<LanguageRow>,
    pub dictionary: Vec<String>,
    pub replacements: Vec<Replacement>,
    pub snippets: Vec<Snippet>,
    pub hardware_summary: String,
    pub recommended_tier: Option<PipelineTier>,
    /// The catalog, each model with how it stands here.
    pub models: Vec<ModelRow>,
    pub models_in_use_summary: String,
    /// The tiers onboarding offers. Custom is one only for someone already on it.
    pub offered_tiers: Vec<TierRow>,
    /// The models the settings use, for onboarding's list.
    pub models_in_use: Vec<ModelRow>,
    /// What that list still has to fetch.
    pub models_to_download: String,
    pub downloaded_summary: String,
    /// Under Downloaded when it's empty.
    pub nothing_downloaded: String,
    pub asr_menu: StageMenu,
    pub vad_menu: StageMenu,
    pub llm_menu: StageMenu,
    pub model_busy: bool,
    pub model_error: String,
    pub models_folder: String,
    pub models_folder_custom: bool,
    pub models_folder_missing: bool,
    pub models_read_only: bool,
    pub pending_folder: String,
    pub pending_summary: String,
    pub moving_models: bool,
    pub move_progress: f32,
    pub data_status: String,
}

/// What the pages show before they've said anything.
#[uniffi::export]
pub fn blank_pages_state() -> PagesState {
    PagesState {
        is_macos: false,
        version: String::new(),
        data_folder: String::new(),
        notice: String::new(),
        notice_page: None,
        vad_forced: false,
        shortcuts: Vec::new(),
        recording_action: None,
        recording_keys: String::new(),
        shortcut_error: String::new(),
        microphones: Vec::new(),
        microphone_auto: true,
        microphone_testing: false,
        microphone_error: String::new(),
        languages_auto: true,
        languages: Vec::new(),
        dictionary: Vec::new(),
        replacements: Vec::new(),
        snippets: Vec::new(),
        hardware_summary: String::new(),
        recommended_tier: None,
        models: Vec::new(),
        models_in_use_summary: String::new(),
        offered_tiers: Vec::new(),
        models_in_use: Vec::new(),
        models_to_download: String::new(),
        downloaded_summary: String::new(),
        nothing_downloaded: String::new(),
        asr_menu: StageMenu::default(),
        vad_menu: StageMenu::default(),
        llm_menu: StageMenu::default(),
        model_busy: false,
        model_error: String::new(),
        models_folder: String::new(),
        models_folder_custom: false,
        models_folder_missing: false,
        models_read_only: false,
        pending_folder: String::new(),
        pending_summary: String::new(),
        moving_models: false,
        move_progress: 0.0,
        data_status: String::new(),
    }
}

/// The record, and what it's worked out from.
pub struct Folded {
    state: PagesState,
    page: SettingsPage,
    facts: Option<PageFacts>,
    values: SettingsValues,
    menus: [Vec<String>; 3],
}

impl Folded {
    pub fn new(page: SettingsPage) -> Self {
        Self {
            state: blank_pages_state(),
            page,
            facts: None,
            values: SettingsValues::default(),
            menus: Default::default(),
        }
    }

    /// The user moved to `page`, so a notice from here on is that page's.
    pub fn set_page(&mut self, page: SettingsPage) {
        self.page = page;
    }

    pub fn take(&mut self, shown: Shown) {
        let state = &mut self.state;
        match shown {
            Shown::Facts(facts) => {
                state.is_macos = facts.is_macos;
                state.version.clone_from(&facts.version);
                state.data_folder.clone_from(&facts.data_folder);
                self.facts = Some(facts);
            }
            Shown::Notice(text) => {
                state.notice_page = (!text.is_empty()).then_some(self.page);
                state.notice = text;
            }
            Shown::Values(values) => self.values = *values,
            Shown::VadForced(forced) => state.vad_forced = forced,
            Shown::Shortcuts(rows) => state.shortcuts = rows,
            Shown::RecordingAction(index) => state.recording_action = u32::try_from(index).ok(),
            Shown::RecordingKeys(keys) => state.recording_keys = keys,
            Shown::ShortcutError(error) => state.shortcut_error = error,
            Shown::Microphones(lines) => state.microphones = microphone_rows(lines),
            Shown::MicrophoneAuto(auto) => state.microphone_auto = auto,
            Shown::MicrophoneTesting(testing) => state.microphone_testing = testing,
            // It crosses on its own, so that it can't hold anything else up.
            Shown::MicrophoneLevel(_) => {}
            Shown::MicrophoneError(error) => state.microphone_error = error,
            Shown::LanguagesAuto(auto) => state.languages_auto = auto,
            Shown::Languages(rows) => state.languages = rows,
            Shown::Dictionary(words) => state.dictionary = words,
            Shown::Replacements(change) => change.apply(&mut state.replacements),
            Shown::Snippets(change) => change.apply(&mut state.snippets),
            Shown::HardwareSummary(summary) => state.hardware_summary = summary,
            Shown::RecommendedTier(tier) => state.recommended_tier = Some(tier),
            Shown::Models(change) => change.apply(&mut state.models),
            Shown::ModelsInUseSummary(summary) => state.models_in_use_summary = summary,
            Shown::DownloadedSummary(summary) => state.downloaded_summary = summary,
            Shown::ModelMenus { asr, vad, llm } => self.menus = [asr, vad, llm],
            Shown::ModelBusy(busy) => state.model_busy = busy,
            Shown::ModelError(error) => state.model_error = error,
            Shown::ModelsFolder(folder) => state.models_folder = folder,
            Shown::ModelsFolderCustom(custom) => state.models_folder_custom = custom,
            Shown::ModelsFolderMissing(missing) => state.models_folder_missing = missing,
            Shown::ModelsReadOnly(read_only) => state.models_read_only = read_only,
            Shown::PendingFolder(folder) => state.pending_folder = folder,
            Shown::PendingSummary(summary) => state.pending_summary = summary,
            Shown::MovingModels(moving) => state.moving_models = moving,
            Shown::MoveProgress(fraction) => state.move_progress = fraction,
            Shown::DataStatus(status) => state.data_status = status,
        }
    }

    /// The record as of now, with what depends on more than one thing worked out.
    pub fn state(&self) -> PagesState {
        let mut state = self.state.clone();
        let needed = [
            true,
            state.vad_forced || self.values.vad,
            self.values.llm_cleanup,
        ];
        let selected = [
            self.values.asr_model,
            self.values.vad_model,
            self.values.llm_model,
        ];
        let rows = self.facts.as_ref().map_or([&[][..]; 3], |facts| {
            [&facts.asr_rows[..], &facts.vad_rows, &facts.llm_rows]
        });
        let [asr, vad, llm] = std::array::from_fn(|stage| {
            let model = usize::try_from(selected[stage])
                .ok()
                .and_then(|option| rows[stage].get(option))
                .and_then(|&row| state.models.get(row as usize))
                .map(|row| stage_model(row.clone(), needed[stage], state.models_folder_missing));
            StageMenu {
                options: self.menus[stage].clone(),
                selected: selected[stage],
                model,
            }
        });
        (state.asr_menu, state.vad_menu, state.llm_menu) = (asr, vad, llm);
        if state.recording_action.is_some() && state.recording_keys.is_empty() {
            "Press keys…".clone_into(&mut state.recording_keys);
        }
        let tier = usize::try_from(self.values.tier).ok();
        let custom = tier.and_then(|tier| TIERS.get(tier)) == Some(&PipelineTier::Custom);
        state.offered_tiers = tier_rows();
        state
            .offered_tiers
            .retain(|row| custom || row.tier != PipelineTier::Custom);
        let in_use = state.models.iter().filter(|row| !row.used_for.is_empty());
        state.models_in_use = in_use.cloned().collect();
        state.models_to_download = if state.models_in_use_summary.is_empty() {
            "Everything is downloaded.".to_owned()
        } else {
            format!("{} to download", state.models_in_use_summary)
        };
        state.nothing_downloaded = if state.models_folder_missing {
            "Connect the folder to see its models."
        } else {
            "Nothing yet. Download a model below, or from the Pipeline page."
        }
        .to_owned();
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lists::Rows;

    fn row(index: i32) -> ModelRow {
        ModelRow {
            index,
            name: format!("model {index}"),
            size: "1 GB".to_owned(),
            status: "Not downloaded".to_owned(),
            ..ModelRow::default()
        }
    }

    /// Three stages, with models 0, 1 and 2 picked.
    fn folded(models: Vec<ModelRow>) -> Folded {
        let mut folded = Folded::new(SettingsPage::Pipeline);
        folded.take(Shown::Facts(PageFacts {
            is_macos: true,
            version: "1".to_owned(),
            data_folder: "/data".to_owned(),
            asr_rows: vec![0],
            vad_rows: vec![1],
            llm_rows: vec![2],
        }));
        folded.take(Shown::Models(Rows::All(models)));
        folded.take(Shown::Values(Box::new(SettingsValues {
            vad: true,
            llm_cleanup: true,
            ..SettingsValues::default()
        })));
        folded
    }

    fn asr_text(model: ModelRow, missing_folder: bool) -> String {
        let mut folded = folded(vec![model, row(1), row(2)]);
        folded.take(Shown::ModelsFolderMissing(missing_folder));
        folded.state().asr_menu.model.unwrap().text
    }

    #[test]
    fn a_stage_says_how_its_model_stands() {
        let installed = ModelRow {
            installed: true,
            ..row(0)
        };
        let partial = ModelRow {
            partial: true,
            status: "300 MB downloaded".to_owned(),
            ..row(0)
        };
        let downloading = ModelRow {
            downloading: true,
            ..row(0)
        };

        assert_eq!(asr_text(installed, false), "Downloaded");
        assert_eq!(asr_text(row(0), false), "Not downloaded · 1 GB");
        assert_eq!(asr_text(partial, false), "300 MB downloaded of 1 GB");
        assert_eq!(asr_text(downloading, false), "Downloading…");
        assert_eq!(asr_text(row(0), true), "Folder not connected");
    }

    #[test]
    fn a_stage_that_is_off_needs_no_model_unless_it_is_forced_on() {
        let mut folded = folded(vec![row(0), row(1), row(2)]);
        folded.take(Shown::Values(Box::default()));

        let state = folded.state();
        let (vad, llm) = (state.vad_menu.model.unwrap(), state.llm_menu.model.unwrap());
        assert_eq!(vad.text, "Not used: this stage is off");
        assert!(!vad.missing && !llm.needed);

        folded.take(Shown::VadForced(true));
        let vad = folded.state().vad_menu.model.unwrap();
        assert!(vad.needed && vad.missing);
    }

    #[test]
    fn a_model_outside_the_menus_shows_no_line() {
        let mut folded = folded(vec![row(0), row(1), row(2)]);
        folded.take(Shown::Values(Box::new(SettingsValues {
            asr_model: -1,
            ..SettingsValues::default()
        })));

        let menu = folded.state().asr_menu;
        assert_eq!(menu.selected, -1);
        assert_eq!(menu.model, None);
    }

    #[test]
    fn a_recording_with_nothing_held_says_to_press_keys() {
        let mut folded = Folded::new(SettingsPage::General);
        assert_eq!(folded.state().recording_keys, "");

        folded.take(Shown::RecordingAction(2));
        assert_eq!(folded.state().recording_action, Some(2));
        assert_eq!(folded.state().recording_keys, "Press keys…");

        folded.take(Shown::RecordingKeys("Ctrl+".to_owned()));
        assert_eq!(folded.state().recording_keys, "Ctrl+");

        folded.take(Shown::RecordingAction(-1));
        assert_eq!(folded.state().recording_action, None);
    }

    #[test]
    fn a_microphone_line_says_its_place_and_whether_it_is_plugged_in() {
        let mut folded = Folded::new(SettingsPage::General);
        let line = |name: &str, connected| MicrophoneLine {
            id: name.to_owned(),
            name: name.to_owned(),
            connected,
        };

        folded.take(Shown::Microphones(vec![
            line("Desk", true),
            line("USB", false),
        ]));

        let titles: Vec<_> = folded
            .state()
            .microphones
            .into_iter()
            .map(|row| row.title)
            .collect();
        assert_eq!(titles, ["1. Desk", "2. USB (not connected)"]);
    }

    #[test]
    fn an_empty_downloaded_list_says_why() {
        let mut folded = Folded::new(SettingsPage::Models);
        assert!(folded.state().nothing_downloaded.starts_with("Nothing yet"));

        folded.take(Shown::ModelsFolderMissing(true));

        assert!(folded.state().nothing_downloaded.starts_with("Connect"));
    }

    #[test]
    fn onboarding_lists_the_models_in_use_and_what_they_still_need() {
        let needed = ModelRow {
            used_for: "Needed for speech".to_owned(),
            ..row(1)
        };
        let mut folded = folded(vec![row(0), needed.clone()]);

        folded.take(Shown::ModelsInUseSummary("1 model, 1 GB".to_owned()));
        assert_eq!(folded.state().models_in_use, [needed]);
        assert_eq!(
            folded.state().models_to_download,
            "1 model, 1 GB to download"
        );

        folded.take(Shown::ModelsInUseSummary(String::new()));
        assert_eq!(
            folded.state().models_to_download,
            "Everything is downloaded."
        );
    }

    #[test]
    fn custom_is_offered_only_to_someone_already_on_it() {
        let titles = |folded: &Folded| -> Vec<String> {
            let rows = folded.state().offered_tiers;
            rows.into_iter().map(|row| row.title).collect()
        };
        let mut folded = Folded::new(SettingsPage::Models);
        assert_eq!(titles(&folded), ["Lite", "Balanced", "High"]);

        folded.take(Shown::Values(Box::new(SettingsValues {
            tier: 3,
            ..SettingsValues::default()
        })));

        assert_eq!(titles(&folded), ["Lite", "Balanced", "High", "Custom"]);
    }

    #[test]
    fn a_notice_belongs_to_the_page_on_show_and_an_empty_one_clears_it() {
        let mut folded = Folded::new(SettingsPage::General);
        folded.set_page(SettingsPage::Models);

        folded.take(Shown::Notice("Couldn't open it".to_owned()));
        assert_eq!(folded.state().notice_page, Some(SettingsPage::Models));

        folded.set_page(SettingsPage::About);
        assert_eq!(folded.state().notice_page, Some(SettingsPage::Models));

        folded.take(Shown::Notice(String::new()));
        assert_eq!(folded.state().notice_page, None);
    }
}
