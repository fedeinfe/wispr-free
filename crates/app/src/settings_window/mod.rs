//! The Settings window (spec §5): created on open, dropped on close. [`Controller`] binds the
//! .slint pages to a [`Pages`], which holds everything they know and do. Closures stored in the UI
//! hold only weak references to it, so dropping the controller frees the window. It drives any
//! [`Surface`], which is how onboarding gets the Models machinery without a second copy.

mod host;

use std::ops::Deref;
use std::rc::Rc;
use std::sync::Arc;

use slint::{ComponentHandle, FilterModel, Model, ModelRc, PlatformError, SharedString, VecModel};
use wf_core::settings::{Replacement, Snippet};
use wf_shell::lists::Rows;
use wf_shell::settings::rows::{
    LanguageRow as Language, ModelRow as ModelLine, ShortcutRow as Shortcut,
};
use wf_shell::settings::values::{SettingsValues as Values, tier_title};
use wf_shell::settings::{Pages, Shown};

use crate::slint_clock::SlintClock;
use crate::{
    LanguageRow, MicrophoneRow, ModelRow, Prefs, ReplacementRow, SettingsValues, SettingsWindow,
    ShortcutRow, SnippetRow,
};

pub use host::{MakeParts, SettingsHost};
pub use wf_shell::settings::system::{self, system_parts};
pub use wf_shell::settings::{
    Desktop, Event, Mailbox, OnChange, Paths, RECORDING_IDLE_LIMIT, SettingsParts, rows, values,
};

/// Fields with the same names on both sides.
#[macro_export]
macro_rules! same_fields {
    ($from:ty => $to:ty { $($field:ident),* $(,)? }) => {
        impl From<$from> for $to {
            #[allow(clippy::useless_conversion)]
            fn from(row: $from) -> Self {
                Self { $($field: row.$field.into()),* }
            }
        }
    };
}

macro_rules! both_ways {
    ($a:ty, $b:ty { $($field:ident),* $(,)? }) => {
        same_fields!($a => $b { $($field),* });
        same_fields!($b => $a { $($field),* });
    };
}

both_ways!(
    Values,
    SettingsValues {
        launch_at_login,
        show_pill,
        show_in_dock,
        tier,
        denoise,
        vad,
        llm_cleanup,
        cleanup_strength,
        vad_auto_stop,
        vad_auto_stop_seconds,
        asr_model,
        vad_model,
        llm_model,
        keep_models_loaded,
        style_personal,
        style_work,
        style_email,
        style_other,
        auto_add_corrections,
        press_enter_command,
        meeting_auto_detect,
        meeting_live_transcript,
        meeting_refine_pass,
        meeting_diarization,
        meeting_keep_audio,
        meeting_max_length_minutes,
        meeting_summary_template,
        sounds,
        media_while_dictating,
        context_awareness,
        history_retention,
    }
);
both_ways!(Replacement, ReplacementRow { wrong, right });
both_ways!(Snippet, SnippetRow { trigger, text });
same_fields!(Shortcut => ShortcutRow { action, combo });
same_fields!(Language => LanguageRow { name, selected });
same_fields!(ModelLine => ModelRow {
    index, name, size, used_for, detail, status, installed, partial, busy, downloading, progress,
});

/// Makes `model` match a list on the toolkit-free side.
pub fn apply_rows<T, Row>(model: &VecModel<Row>, change: Rows<T>)
where
    Row: From<T> + Clone + 'static,
{
    match change {
        Rows::All(rows) => model.set_vec(rows.into_iter().map(Row::from).collect::<Vec<_>>()),
        Rows::One(index, row) => model.set_row_data(index, row.into()),
        Rows::Add(row) => model.push(row.into()),
        Rows::Insert(index, row) => model.insert(index, row.into()),
    }
}

pub fn list_model<T, Row>(rows: Vec<T>) -> ModelRc<Row>
where
    Row: From<T> + Clone + 'static,
{
    ModelRc::new(VecModel::from(
        rows.into_iter().map(Row::from).collect::<Vec<_>>(),
    ))
}

/// A window with the Settings pages' state in its `Prefs`.
pub trait Surface: ComponentHandle + Sized + 'static {
    fn create() -> Result<Self, PlatformError>;
    fn prefs(&self) -> Prefs<'_>;
}

impl Surface for SettingsWindow {
    fn create() -> Result<Self, PlatformError> {
        Self::new()
    }

    fn prefs(&self) -> Prefs<'_> {
        self.global::<Prefs>()
    }
}

/// The lists shared with `Prefs`, which rows change in place.
#[derive(Default)]
struct Lists {
    models: Rc<VecModel<ModelRow>>,
    replacements: Rc<VecModel<ReplacementRow>>,
    snippets: Rc<VecModel<SnippetRow>>,
}

/// One open window. Dropping it closes everything the window started.
pub struct Controller<W: Surface> {
    pages: Rc<Pages>,
    window: W,
}

pub type SettingsController = Controller<SettingsWindow>;

/// Connects a `Prefs` callback to a method of [`Pages`] through a weak reference.
macro_rules! wire {
    ($pages:ident, $prefs:ident . $on:ident, |$this:ident $(, $arg:ident)*| $body:expr) => {{
        let weak = Rc::downgrade($pages);
        $prefs.$on(move |$($arg),*| {
            if let Some($this) = weak.upgrade() {
                $body
            }
        });
    }};
}

impl<W: Surface> Controller<W> {
    pub fn new(parts: SettingsParts) -> Result<Self, PlatformError> {
        let window = W::create()?;
        let lists = Rc::new(Lists::default());
        bind_lists(&window.prefs(), &lists);
        let weak = window.as_weak();
        let view = Rc::new(move |shown| {
            if let Some(window) = weak.upgrade() {
                show(&window.prefs(), &lists, shown);
            }
        });
        let weak = window.as_weak();
        let wake = move || {
            let _ = weak.upgrade_in_event_loop(|window| window.prefs().invoke_events_arrived());
        };
        let pages = Pages::new(parts, view, &SlintClock, Arc::new(wake));
        connect(&window, &pages);
        Ok(Self { pages, window })
    }

    pub fn window(&self) -> &W {
        &self.window
    }

    pub fn pages(&self) -> &Rc<Pages> {
        &self.pages
    }
}

impl<W: Surface> Deref for Controller<W> {
    type Target = Pages;

    fn deref(&self) -> &Pages {
        &self.pages
    }
}

fn bind_lists(prefs: &Prefs<'_>, lists: &Lists) {
    prefs.set_models(ModelRc::from(Rc::clone(&lists.models)));
    prefs.set_downloaded_models(ModelRc::new(FilterModel::new(
        Rc::clone(&lists.models),
        |row: &ModelRow| row.installed,
    )));
    prefs.set_available_models(ModelRc::new(FilterModel::new(
        Rc::clone(&lists.models),
        |row: &ModelRow| !row.installed,
    )));
    prefs.set_models_in_use(ModelRc::new(FilterModel::new(
        Rc::clone(&lists.models),
        |row: &ModelRow| !row.used_for.is_empty(),
    )));
    prefs.set_replacements(ModelRc::from(Rc::clone(&lists.replacements)));
    prefs.set_snippets(ModelRc::from(Rc::clone(&lists.snippets)));
}

fn connect<W: Surface>(window: &W, pages: &Rc<Pages>) {
    let prefs = window.prefs();
    let values = window.as_weak();
    wire!(pages, prefs.on_commit, |this| {
        if let Some(window) = values.upgrade() {
            this.commit(&window.prefs().get_values().into());
        }
    });
    wire!(pages, prefs.on_events_arrived, |this| this.drain_events());
    wire!(pages, prefs.on_record_shortcut, |this, index| this
        .record_shortcut(index));
    wire!(pages, prefs.on_stop_recording, |this| this.stop_recording());
    wire!(pages, prefs.on_reset_shortcut, |this, index| this
        .reset_shortcut(index));
    wire!(pages, prefs.on_set_microphone_auto, |this, auto| this
        .set_microphone_auto(auto));
    wire!(pages, prefs.on_move_microphone, |this, index, by| this
        .move_microphone(index, by));
    wire!(pages, prefs.on_toggle_microphone_test, |this| this
        .toggle_microphone_test());
    wire!(pages, prefs.on_set_languages_auto, |this, auto| this
        .set_languages_auto(auto));
    wire!(pages, prefs.on_toggle_language, |this, index| this
        .toggle_language(index));
    wire!(pages, prefs.on_download_model, |this, index| this
        .download_model(index));
    wire!(pages, prefs.on_download_models_in_use, |this| this
        .download_models_in_use());
    wire!(pages, prefs.on_cancel_download, |this| this
        .cancel_download());
    wire!(pages, prefs.on_delete_model, |this, index| this
        .delete_model(index));
    wire!(pages, prefs.on_verify_model, |this, index| this
        .verify_model(index));
    wire!(pages, prefs.on_add_word, |this, word| this.add_word(&word));
    wire!(pages, prefs.on_remove_word, |this, index| this
        .remove_word(index));
    wire!(pages, prefs.on_add_replacement, |this| this
        .add_replacement());
    wire!(pages, prefs.on_edit_replacement, |this, index, row| this
        .edit_replacement(index as usize, row.into()));
    wire!(pages, prefs.on_remove_replacement, |this, index| this
        .remove_replacement(index as usize));
    wire!(pages, prefs.on_add_snippet, |this| this.add_snippet());
    wire!(pages, prefs.on_edit_snippet, |this, index, row| this
        .edit_snippet(index as usize, row.into()));
    wire!(pages, prefs.on_remove_snippet, |this, index| this
        .remove_snippet(index as usize));
    wire!(pages, prefs.on_choose_models_folder, |this| this
        .choose_models_folder());
    wire!(pages, prefs.on_use_default_models_folder, |this| this
        .use_default_models_folder());
    wire!(pages, prefs.on_open_models_folder, |this| this
        .open_models_folder());
    wire!(pages, prefs.on_recheck_models_folder, |this| this
        .recheck_models_folder());
    wire!(pages, prefs.on_move_to_pending_folder, |this| this
        .move_to_pending_folder());
    wire!(pages, prefs.on_switch_to_pending_folder, |this| this
        .switch_to_pending_folder());
    wire!(pages, prefs.on_cancel_pending_folder, |this| this
        .cancel_pending_folder());
    wire!(pages, prefs.on_cancel_move, |this| this.cancel_move());
    wire!(pages, prefs.on_open_data_folder, |this| this
        .open_data_folder());
    wire!(pages, prefs.on_export_all, |this| this.export_all());
    wire!(pages, prefs.on_delete_all, |this| this.delete_all());
    wire!(pages, prefs.on_check_for_updates, |this| this
        .check_for_updates());
}

fn show(prefs: &Prefs<'_>, lists: &Lists, shown: Shown) {
    match shown {
        Shown::Facts(facts) => {
            prefs.set_is_macos(facts.is_macos);
            prefs.set_version(facts.version.into());
            prefs.set_data_folder(facts.data_folder.into());
            prefs.set_asr_rows(ModelRc::new(VecModel::from(facts.asr_rows)));
            prefs.set_vad_rows(ModelRc::new(VecModel::from(facts.vad_rows)));
            prefs.set_llm_rows(ModelRc::new(VecModel::from(facts.llm_rows)));
        }
        Shown::Notice(text) => prefs.set_notice(text.into()),
        Shown::Values(values) => {
            let values = SettingsValues::from(*values);
            if prefs.get_values() != values {
                prefs.set_values(values);
            }
        }
        Shown::VadForced(forced) => prefs.set_vad_forced(forced),
        Shown::Shortcuts(rows) => prefs.set_shortcuts(list_model(rows)),
        Shown::RecordingAction(index) => prefs.set_recording_action(index),
        Shown::RecordingKeys(keys) => prefs.set_recording_keys(keys.into()),
        Shown::ShortcutError(error) => prefs.set_shortcut_error(error.into()),
        Shown::Microphones(lines) => {
            let rows = lines.into_iter().map(|line| MicrophoneRow {
                name: line.name.into(),
                connected: line.connected,
            });
            prefs.set_microphones(ModelRc::new(VecModel::from(rows.collect::<Vec<_>>())));
        }
        Shown::MicrophoneAuto(auto) => prefs.set_microphone_auto(auto),
        Shown::MicrophoneTesting(testing) => prefs.set_microphone_testing(testing),
        Shown::MicrophoneLevel(level) => prefs.set_microphone_level(level),
        Shown::MicrophoneError(error) => prefs.set_microphone_error(error.into()),
        Shown::LanguagesAuto(auto) => prefs.set_languages_auto(auto),
        Shown::Languages(rows) => prefs.set_languages(list_model(rows)),
        Shown::Dictionary(words) => prefs.set_dictionary(string_model(words)),
        Shown::Replacements(change) => apply_rows(&lists.replacements, change),
        Shown::Snippets(change) => apply_rows(&lists.snippets, change),
        Shown::HardwareSummary(summary) => prefs.set_hardware_summary(summary.into()),
        Shown::RecommendedTier(tier) => prefs.set_recommended_tier(tier_title(tier).into()),
        Shown::Models(change) => apply_rows(&lists.models, change),
        Shown::ModelsInUseSummary(summary) => prefs.set_models_in_use_summary(summary.into()),
        Shown::DownloadedSummary(summary) => prefs.set_downloaded_summary(summary.into()),
        Shown::ModelMenus { asr, vad, llm } => {
            prefs.set_asr_options(string_model(asr));
            prefs.set_vad_options(string_model(vad));
            prefs.set_llm_options(string_model(llm));
        }
        Shown::ModelBusy(busy) => prefs.set_model_busy(busy),
        Shown::ModelError(error) => prefs.set_model_error(error.into()),
        Shown::ModelsFolder(folder) => prefs.set_models_folder(folder.into()),
        Shown::ModelsFolderCustom(custom) => prefs.set_models_folder_custom(custom),
        Shown::ModelsFolderMissing(missing) => prefs.set_models_folder_missing(missing),
        // The Slint app has no fence.
        Shown::ModelsReadOnly(_) => {}
        Shown::PendingFolder(folder) => prefs.set_pending_folder(folder.into()),
        Shown::PendingSummary(summary) => prefs.set_pending_summary(summary.into()),
        Shown::MovingModels(moving) => prefs.set_moving_models(moving),
        Shown::MoveProgress(fraction) => prefs.set_move_progress(fraction),
        Shown::DataStatus(status) => prefs.set_data_status(status.into()),
    }
}

fn string_model(items: Vec<String>) -> ModelRc<SharedString> {
    let items: Vec<SharedString> = items.into_iter().map(Into::into).collect();
    ModelRc::new(VecModel::from(items))
}
