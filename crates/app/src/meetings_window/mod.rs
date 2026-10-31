//! The Meetings window (spec §4): created on open, dropped on close. [`MeetingsController`] binds
//! the .slint to a [`Meetings`], which holds everything it knows and does.

mod host;

use std::ops::Deref;
use std::rc::Rc;
use std::sync::Arc;

use slint::{ComponentHandle, ModelRc, PlatformError, SharedString, VecModel};
use wf_shell::lists::Rows;
use wf_shell::meetings::rows as shell;
use wf_shell::meetings::{Meetings, Shown};

use crate::settings_window::{apply_rows, list_model};
use crate::slint_clock::SlintClock;
use crate::{
    ChatBlock, ChatChip, ChatMessage, MeetingRow, MeetingsUi, MeetingsWindow, Prefs, SpeakerRow,
    StageRow, SummaryLine, TranscriptLine,
};

pub use host::{MakeParts, MeetingsBoard, MeetingsHost};
pub use wf_shell::meetings::{Event, MeetingActions, MeetingsParts, rows};

crate::same_fields!(shell::MeetingRow => MeetingRow {
    id, title, when, snippet, hit_row, recording, busy, failed,
});
crate::same_fields!(shell::StageRow => StageRow { label, state, reason });
crate::same_fields!(shell::SummaryLine => SummaryLine { kind, text, time, row });
crate::same_fields!(shell::TranscriptLine => TranscriptLine { time, speaker, text, gap });
crate::same_fields!(shell::SpeakerRow => SpeakerRow { id, label, name });
crate::same_fields!(shell::ChatChip => ChatChip { label, row });

impl From<shell::ChatBlock> for ChatBlock {
    fn from(block: shell::ChatBlock) -> Self {
        Self {
            text: block.text.into(),
            chips: list_model(block.chips),
        }
    }
}

impl From<shell::ChatMessage> for ChatMessage {
    fn from(message: shell::ChatMessage) -> Self {
        Self {
            question: message.question.into(),
            blocks: list_model(message.blocks),
            note: message.note.into(),
        }
    }
}

/// The lists shared with `MeetingsUi`, which rows change in place.
#[derive(Default)]
struct Lists {
    meetings: Rc<VecModel<MeetingRow>>,
    summary: Rc<VecModel<SummaryLine>>,
    transcript: Rc<VecModel<TranscriptLine>>,
    speakers: Rc<VecModel<SpeakerRow>>,
    stages: Rc<VecModel<StageRow>>,
    chat: Rc<VecModel<ChatMessage>>,
}

/// One open Meetings window. Dropping it saves what's unsaved and stops what it started.
pub struct MeetingsController {
    meetings: Rc<Meetings>,
    window: MeetingsWindow,
}

/// Connects a `MeetingsUi` callback to a method of [`Meetings`] through a weak reference.
macro_rules! wire {
    ($meetings:ident, $ui:ident . $on:ident, |$this:ident $(, $arg:ident)*| $body:expr) => {{
        let weak = Rc::downgrade($meetings);
        $ui.$on(move |$($arg),*| {
            if let Some($this) = weak.upgrade() {
                $body
            }
        });
    }};
}

impl MeetingsController {
    pub fn new(parts: MeetingsParts) -> Result<Self, PlatformError> {
        let window = MeetingsWindow::new()?;
        window.global::<Prefs>().set_theme(parts.theme);

        let lists = Rc::new(Lists::default());
        let ui = window.global::<MeetingsUi>();
        ui.set_meetings(ModelRc::from(Rc::clone(&lists.meetings)));
        ui.set_summary(ModelRc::from(Rc::clone(&lists.summary)));
        ui.set_transcript(ModelRc::from(Rc::clone(&lists.transcript)));
        ui.set_speakers(ModelRc::from(Rc::clone(&lists.speakers)));
        ui.set_stages(ModelRc::from(Rc::clone(&lists.stages)));
        ui.set_chat(ModelRc::from(Rc::clone(&lists.chat)));

        let weak = window.as_weak();
        let view = Rc::new(move |shown| {
            if let Some(window) = weak.upgrade() {
                show(&window.global::<MeetingsUi>(), &lists, shown);
            }
        });
        let weak = window.as_weak();
        let wake = move || {
            let _ = weak.upgrade_in_event_loop(|window| {
                window.global::<MeetingsUi>().invoke_events_arrived();
            });
        };
        let meetings = Meetings::new(parts, view, Rc::new(SlintClock), Arc::new(wake))
            .map_err(PlatformError::Other)?;
        connect(&window, &meetings);
        Ok(Self { meetings, window })
    }

    pub fn window(&self) -> &MeetingsWindow {
        &self.window
    }
}

impl Deref for MeetingsController {
    type Target = Meetings;

    fn deref(&self) -> &Meetings {
        &self.meetings
    }
}

fn connect(window: &MeetingsWindow, meetings: &Rc<Meetings>) {
    let ui = window.global::<MeetingsUi>();
    wire!(meetings, ui.on_events_arrived, |this| this.drain_events());
    wire!(meetings, ui.on_toggle_meeting, |this| this.toggle_meeting());
    let page = window.as_weak();
    wire!(meetings, ui.on_search, |this| {
        if let Some(window) = page.upgrade() {
            let query = window.global::<MeetingsUi>().get_query();
            this.search(query.to_string());
        }
    });
    wire!(meetings, ui.on_select, |this, id, row| this
        .select(&id, row));
    let page = window.as_weak();
    wire!(meetings, ui.on_title_edited, |this| {
        if let Some(window) = page.upgrade() {
            let title = window.global::<MeetingsUi>().get_title();
            this.title_edited(title.to_string());
        }
    });
    let page = window.as_weak();
    wire!(meetings, ui.on_notes_edited, |this| {
        if let Some(window) = page.upgrade() {
            let notes = window.global::<MeetingsUi>().get_notes();
            this.notes_edited(notes.to_string());
        }
    });
    wire!(meetings, ui.on_speaker_edited, |this, speaker, name| this
        .speaker_edited(speaker, name.to_string()));
    wire!(meetings, ui.on_export, |this| this.export());
    wire!(meetings, ui.on_delete, |this| this.delete());
    wire!(meetings, ui.on_resume, |this| this.resume());
    wire!(meetings, ui.on_finish, |this| this.finish());
    wire!(meetings, ui.on_cancel_work, |this| this.cancel_work());
    wire!(meetings, ui.on_regenerate, |this| this.regenerate());
    wire!(meetings, ui.on_update_summary, |this| this.update_summary());
    wire!(meetings, ui.on_ask, |this, question| this.ask(&question));
    wire!(meetings, ui.on_stop_chat, |this| this.stop_chat());
}

fn show(ui: &MeetingsUi<'_>, lists: &Lists, shown: Shown) {
    match shown {
        Shown::Meetings(change) => apply_rows(&lists.meetings, change),
        Shown::ListHint(hint) => ui.set_list_hint(hint.into()),
        Shown::Recording(recording) => ui.set_recording(recording),
        Shown::SelectedId(id) => ui.set_selected_id(id.into()),
        Shown::Title(title) => ui.set_title(title.into()),
        Shown::Notes(notes) => ui.set_notes(notes.into()),
        Shown::When(when) => ui.set_when(when.into()),
        Shown::ThisRecording(recording) => ui.set_this_recording(recording),
        Shown::Busy(busy) => ui.set_busy(busy),
        Shown::CanFinish(can) => ui.set_can_finish(can),
        Shown::CanResume(can) => ui.set_can_resume(can),
        Shown::CanSummarize(can) => ui.set_can_summarize(can),
        Shown::Stages(rows) => {
            apply_rows(&lists.stages, Rows::All(rows));
        }
        Shown::Failures(failures) => ui.set_failures(failures.into()),
        Shown::ProgressText(text) => ui.set_progress_text(text.into()),
        Shown::Progress(progress) => ui.set_progress(progress),
        Shown::Notice(text) => ui.set_notice(text.into()),
        Shown::Tab(tab) => ui.set_tab(tab),
        Shown::Jump(row) => ui.invoke_jump(row),
        Shown::Summary(lines) => {
            apply_rows(&lists.summary, Rows::All(lines));
        }
        Shown::SummaryEmpty(text) => ui.set_summary_empty(text.into()),
        Shown::SummaryPartial(partial) => ui.set_summary_partial(partial),
        Shown::SummaryStale(stale) => ui.set_summary_stale(stale),
        Shown::Transcript(change) => apply_rows(&lists.transcript, change),
        Shown::TranscriptEmpty(text) => ui.set_transcript_empty(text.into()),
        Shown::Speakers(rows) => {
            apply_rows(&lists.speakers, Rows::All(rows));
        }
        Shown::Chat(change) => apply_rows(&lists.chat, change),
        Shown::Suggestions(questions) => {
            let rows: Vec<SharedString> = questions.into_iter().map(Into::into).collect();
            ui.set_suggestions(ModelRc::new(VecModel::from(rows)));
        }
        Shown::ChatRunning(running) => ui.set_chat_running(running),
        Shown::StreamingQuestion(question) => ui.set_streaming_question(question.into()),
        Shown::StreamingText(text) => ui.set_streaming_text(text.into()),
        Shown::ChatError(error) => ui.set_chat_error(error.into()),
    }
}
