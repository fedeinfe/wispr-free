//! The Meetings window (spec §4), with no window. The meeting data and what's done with it live
//! in `wf_meetings` and `wf_dictation`. [`Meetings`] ties them to whatever draws them through a
//! [`View`], with disk work on [`disk`]'s thread and engine work in the flow, so the UI thread never
//! waits.

pub mod changes;
pub mod desk;
mod disk;
pub mod rows;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use wf_core::settings::data_dir;
use wf_dictation::{ChatRun, ChatUpdate, Dictation, MeetingNews, MeetingsStatus, SummaryMode};
use wf_meetings::{
    LiveEvent, MeetingDetail, MeetingEntry, MeetingStore, SearchHit, SpeakerId, Transcript,
    TranscriptRow, suggested_questions,
};
use wf_platform::desktop;

use crate::clock::{Clock, Timer};
use crate::lists::Rows;
use crate::settings::system::export_folder;
use disk::Task;
use rows::{
    ChatMessage, MeetingRow, SpeakerRow, StageRow, SummaryLine, TranscriptLine, chat_message,
    failures_text, meeting_row, speaker_rows, stage_rows, step_text, summary_lines,
    transcript_line, when_text,
};

/// How long after the last key an edit goes to disk.
const SAVE_AFTER: Duration = Duration::from_millis(700);
/// How long after the last key a search runs.
const SEARCH_AFTER: Duration = Duration::from_millis(150);
/// Enough for the frame that makes a just-opened meeting's rows.
const ROWS_READY: Duration = Duration::from_millis(50);
const TRANSCRIPT_TAB: i32 = 2;

const EMPTY_LIST: &str = "No meetings yet. Start one here, from the tray, or with the shortcut.";
const NO_HITS: &str = "Nothing matches that.";
const PICK_ONE: &str = "Pick a meeting to see its summary, notes, transcript and chat.";
const START: &str = "Start meeting notes";
const STOP: &str = "Stop meeting notes";
const ASK_HINT: &str =
    "Answers come only from this meeting, and each one shows the times it rests on.";
const NO_SUMMARY: &str = "No summary yet.";
const NO_TRANSCRIPT: &str = "Nothing was transcribed.";
const LISTENING: &str = "Listening. The first lines take a few seconds.";

/// What the window asks of the running app, behind a seam so tests record nothing.
pub trait MeetingActions {
    fn toggle(&self);
    fn resume(&self, id: &str);
    fn finish(&self, id: &str);
    fn summarize(&self, id: &str, mode: SummaryMode);
    fn cancel(&self, id: &str);
    fn ask(&self, id: &str, question: &str) -> ChatRun;
    fn status(&self) -> MeetingsStatus;
}

impl MeetingActions for Dictation {
    fn toggle(&self) {
        self.toggle_meeting();
    }

    fn resume(&self, id: &str) {
        self.resume_meeting(id);
    }

    fn finish(&self, id: &str) {
        self.finish_meeting(id);
    }

    fn summarize(&self, id: &str, mode: SummaryMode) {
        self.summarize_meeting(id, mode);
    }

    fn cancel(&self, id: &str) {
        self.cancel_meeting_work(id);
    }

    fn ask(&self, id: &str, question: &str) -> ChatRun {
        self.ask_meeting(id, question)
    }

    fn status(&self) -> MeetingsStatus {
        self.meetings_status()
    }
}

/// Everything a window is built from.
#[derive(Clone)]
pub struct MeetingsParts {
    pub store: MeetingStore,
    pub actions: Rc<dyn MeetingActions>,
    /// As the windows' `Prefs.values` hold it.
    pub theme: i32,
    /// Where exports go.
    pub export_dir: PathBuf,
    /// Gives an opened window the foreground, which an app without a Dock icon doesn't get.
    pub bring_to_front: Rc<dyn Fn()>,
}

/// What a window on the real machine is built from, each time one opens.
pub fn system_parts(
    data: &Path,
    export_dir: &Path,
    theme: i32,
    actions: Dictation,
) -> MeetingsParts {
    MeetingsParts {
        store: MeetingStore::new(data.join("meetings")),
        actions: Rc::new(actions),
        theme,
        export_dir: export_dir.to_path_buf(),
        bring_to_front: Rc::new(desktop::activate_app),
    }
}

/// The data folder and where exports go, on the real machine.
pub fn system_folders() -> (PathBuf, PathBuf) {
    let data = data_dir().unwrap_or_else(|| PathBuf::from("wispr-free"));
    let export_dir = export_folder(&data);
    (data, export_dir)
}

/// What the disk thread reports to the window.
#[derive(Debug)]
pub enum Event {
    Listed(Vec<MeetingEntry>),
    Found {
        query: String,
        hits: Vec<SearchHit>,
    },
    Opened(Box<MeetingDetail>),
    /// The list needs reading again.
    Changed,
    /// A speaker has a new name, which shows all through the meeting.
    Renamed(String),
    Deleted(String),
    Exported(PathBuf),
    Failed(String),
}

/// One thing for the window to show now.
#[derive(Debug, Clone, PartialEq)]
pub enum Shown {
    Meetings(Rows<MeetingRow>),
    ListHint(String),
    /// What shows with nothing picked.
    DetailHint(String),
    Recording(bool),
    /// What the button that starts and stops a meeting says.
    StartLabel(String),
    SelectedId(String),
    Title(String),
    Notes(String),
    When(String),
    ThisRecording(bool),
    Busy(bool),
    CanFinish(bool),
    CanResume(bool),
    CanSummarize(bool),
    CanExport(bool),
    CanDelete(bool),
    Stages(Vec<StageRow>),
    Failures(String),
    ProgressText(String),
    /// 0..1, or below 0 when that can't be said.
    Progress(f32),
    Notice(String),
    Tab(i32),
    /// Shows a line of the transcript.
    Jump(i32),
    Summary(Vec<SummaryLine>),
    SummaryEmpty(String),
    SummaryPartial(bool),
    SummaryStale(bool),
    Transcript(Rows<TranscriptLine>),
    TranscriptEmpty(String),
    Speakers(Vec<SpeakerRow>),
    Chat(Rows<ChatMessage>),
    Suggestions(Vec<String>),
    /// Shows until something has been asked.
    ChatHint(String),
    ChatRunning(bool),
    StreamingQuestion(String),
    StreamingText(String),
    ChatError(String),
}

/// Where the window says what to show. Takes no calls back in.
pub type View = Rc<dyn Fn(Shown)>;

/// Delivers [`Event`]s to the window from any thread. Dropped silently once the window is gone.
#[derive(Clone)]
pub struct Mailbox {
    events: Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Mailbox {
    pub fn post(&self, event: Event) {
        if self.events.send(event).is_ok() {
            (self.wake)();
        }
    }
}

/// Edits not on disk yet.
#[derive(Default)]
struct Edits {
    notes: Option<(String, String)>,
    title: Option<(String, String)>,
    speakers: Vec<(String, SpeakerId, String)>,
}

/// An answer on its way.
struct Asking {
    run: ChatRun,
    id: String,
    question: String,
    text: String,
}

/// One open Meetings window. Dropping it saves what's unsaved and stops what it started.
pub struct Meetings {
    me: Weak<Meetings>,
    view: View,
    clock: Rc<dyn Clock>,
    parts: MeetingsParts,
    tasks: Sender<Task>,
    events: Receiver<Event>,
    list: RefCell<Vec<MeetingRow>>,
    selected: RefCell<Option<String>>,
    query: RefCell<String>,
    entries: RefCell<Vec<MeetingEntry>>,
    /// What the search box last found. `None` shows every meeting.
    hits: RefCell<Option<Vec<SearchHit>>>,
    detail: RefCell<Option<MeetingDetail>>,
    /// The line to show once the meeting being opened is there.
    jump_when_open: Cell<i32>,
    asking: RefCell<Option<Asking>>,
    edits: RefCell<Edits>,
    save_timer: Box<dyn Timer>,
    search_timer: Box<dyn Timer>,
}

impl Meetings {
    /// `wake` runs on any thread when events arrive, and must end in [`Meetings::drain_events`] on
    /// the UI thread.
    pub fn new(
        parts: MeetingsParts,
        view: View,
        clock: Rc<dyn Clock>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Rc<Self>, String> {
        let (sender, events) = mpsc::channel();
        let mailbox = Mailbox {
            events: sender,
            wake,
        };
        let (tasks, queue) = mpsc::channel();
        let (store, export_dir) = (parts.store.clone(), parts.export_dir.clone());
        thread::Builder::new()
            .name("wf-meetings-disk".into())
            .spawn(move || disk::run(&store, &export_dir, queue, &mailbox))
            .map_err(|error| error.to_string())?;
        let (save_timer, search_timer) = (clock.timer(), clock.timer());
        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            view,
            clock,
            parts,
            tasks,
            events,
            list: RefCell::default(),
            selected: RefCell::default(),
            query: RefCell::default(),
            entries: RefCell::default(),
            hits: RefCell::default(),
            detail: RefCell::default(),
            jump_when_open: Cell::new(-1),
            asking: RefCell::default(),
            edits: RefCell::default(),
            save_timer,
            search_timer,
        });
        this.show_status();
        this.send(Task::List);
        Ok(this)
    }

    fn show(&self, shown: Shown) {
        (self.view)(shown);
    }

    pub fn bring_to_front(&self) {
        (self.parts.bring_to_front)();
    }

    fn send(&self, task: Task) {
        let _ = self.tasks.send(task);
    }

    fn selected(&self) -> Option<String> {
        self.selected.borrow().clone()
    }

    fn select_id(&self, id: Option<String>) {
        self.show(Shown::SelectedId(id.clone().unwrap_or_default()));
        *self.selected.borrow_mut() = id;
    }

    /// Runs what the disk thread has reported so far.
    pub fn drain_events(&self) {
        while let Ok(event) = self.events.try_recv() {
            self.handle(event);
        }
    }

    pub fn toggle_meeting(&self) {
        self.parts.actions.toggle();
    }

    pub fn title_edited(&self, title: String) {
        if let Some(id) = self.selected() {
            self.edits.borrow_mut().title = Some((id, title));
            self.save_soon();
        }
    }

    pub fn notes_edited(&self, notes: String) {
        if let Some(id) = self.selected() {
            self.edits.borrow_mut().notes = Some((id, notes));
            self.save_soon();
        }
    }

    pub fn speaker_edited(&self, speaker: i32, name: String) {
        if let Some(id) = self.selected() {
            let speaker = SpeakerId(speaker as u32);
            let mut edits = self.edits.borrow_mut();
            edits.speakers.retain(|(_, known, _)| *known != speaker);
            edits.speakers.push((id, speaker, name));
            drop(edits);
            self.save_soon();
        }
    }

    pub fn resume(&self) {
        if let Some(id) = self.selected() {
            self.flush_edits();
            self.parts.actions.resume(&id);
        }
    }

    pub fn finish(&self) {
        if let Some(id) = self.selected() {
            self.parts.actions.finish(&id);
        }
    }

    pub fn cancel_work(&self) {
        if let Some(id) = self.selected() {
            self.parts.actions.cancel(&id);
        }
    }

    pub fn regenerate(&self) {
        self.summarize(SummaryMode::Regenerate);
    }

    pub fn update_summary(&self) {
        self.summarize(SummaryMode::Update);
    }

    pub fn stop_chat(&self) {
        if let Some(asking) = &*self.asking.borrow() {
            asking.run.stop();
        }
    }

    fn handle(&self, event: Event) {
        match event {
            Event::Listed(entries) => {
                *self.entries.borrow_mut() = entries;
                self.show_list();
                let query = self.query.borrow().clone();
                if !query.trim().is_empty() {
                    self.send(Task::Search(query));
                }
            }
            Event::Found { query, hits } => {
                if *self.query.borrow() == query {
                    *self.hits.borrow_mut() = Some(hits);
                    self.show_list();
                }
            }
            Event::Opened(detail) => self.show_detail(*detail),
            Event::Changed => self.send(Task::List),
            Event::Renamed(id) => {
                if self.selected().as_ref() == Some(&id) {
                    self.send(Task::Open(id));
                }
            }
            Event::Deleted(id) => {
                if self.selected().as_ref() == Some(&id) {
                    self.clear_selection();
                }
                self.send(Task::List);
            }
            Event::Exported(path) => self.notice(&format!("Saved to {}", path.display())),
            Event::Failed(problem) => self.notice(&problem),
        }
    }

    fn notice(&self, text: &str) {
        self.show(Shown::Notice(text.to_owned()));
    }

    /// Takes in what the flow reported. On the UI thread.
    pub fn news(&self, news: MeetingNews) {
        match news {
            MeetingNews::ListChanged => self.send(Task::List),
            MeetingNews::Recording(id) => {
                self.send(Task::List);
                if self.selected().is_none() {
                    self.select(&id, -1);
                } else if self.selected().as_ref() == Some(&id) {
                    self.send(Task::Open(id));
                }
                self.show_status();
            }
            MeetingNews::Stopped(id) => {
                self.send(Task::List);
                self.show_status();
                self.reopen(id);
            }
            MeetingNews::Live { id, event } => self.live(&id, event),
            MeetingNews::Working { .. } => self.show_status(),
            MeetingNews::WorkDone { id, problem } => {
                self.send(Task::List);
                self.show_status();
                if self.selected().as_ref() == Some(&id)
                    && let Some(problem) = problem
                {
                    self.notice(&problem);
                }
                self.reopen(id);
            }
            MeetingNews::Chat { run, update } => self.chat_update(run, update),
        }
    }

    fn reopen(&self, id: String) {
        if self.selected().as_ref() == Some(&id) {
            self.send(Task::Open(id));
        }
    }

    /// The search box changed.
    pub fn search(&self, query: String) {
        *self.query.borrow_mut() = query;
        let weak = self.me.clone();
        self.search_timer.once(
            SEARCH_AFTER,
            Box::new(move || {
                if let Some(this) = weak.upgrade() {
                    let query = this.query.borrow().clone();
                    if query.trim().is_empty() {
                        this.hits.borrow_mut().take();
                        this.show_list();
                    } else {
                        this.send(Task::Search(query));
                    }
                }
            }),
        );
    }

    fn save_soon(&self) {
        let weak = self.me.clone();
        self.save_timer.once(
            SAVE_AFTER,
            Box::new(move || {
                if let Some(this) = weak.upgrade() {
                    this.flush_edits();
                }
            }),
        );
    }

    fn flush_edits(&self) {
        self.save_timer.stop();
        let mut edits = self.edits.borrow_mut();
        if let Some((id, text)) = edits.notes.take() {
            self.send(Task::SaveNotes { id, text });
        }
        if let Some((id, title)) = edits.title.take() {
            self.send(Task::SaveTitle { id, title });
        }
        for (id, speaker, name) in edits.speakers.drain(..) {
            self.send(Task::NameSpeaker { id, speaker, name });
        }
    }

    fn change_list(&self, change: Rows<MeetingRow>) {
        change.clone().apply(&mut self.list.borrow_mut());
        self.show(Shown::Meetings(change));
    }

    fn show_list(&self) {
        let status = self.parts.actions.status();
        let entries = self.entries.borrow();
        let hits = self.hits.borrow();
        let rows: Vec<MeetingRow> = match &*hits {
            Some(hits) => hits
                .iter()
                .filter_map(|hit| {
                    let entry = entries.iter().find(|entry| entry.id == hit.id)?;
                    Some(meeting_row(entry, &status, Some(hit)))
                })
                .collect(),
            None => entries
                .iter()
                .map(|entry| meeting_row(entry, &status, None))
                .collect(),
        };
        let hint = if hits.is_some() { NO_HITS } else { EMPTY_LIST };
        self.show(Shown::ListHint(hint.to_owned()));
        let pick = if rows.is_empty() { "" } else { PICK_ONE };
        self.show(Shown::DetailHint(pick.to_owned()));
        self.change_list(Rows::All(rows));
    }

    /// Opens a meeting, at line `row` of its transcript if that isn't -1.
    pub fn select(&self, id: &str, row: i32) {
        self.flush_edits();
        if self.selected().as_deref() != Some(id) {
            self.select_id(Some(id.to_owned()));
            self.show(Shown::Notice(String::new()));
            self.detail.borrow_mut().take();
            self.show(Shown::Transcript(Rows::All(Vec::new())));
            self.show_status();
        }
        self.jump_when_open.set(row);
        self.send(Task::Open(id.to_owned()));
    }

    fn clear_selection(&self) {
        self.select_id(None);
        self.detail.borrow_mut().take();
        self.show_status();
    }

    fn show_detail(&self, detail: MeetingDetail) {
        if self.selected().as_ref() != Some(&detail.record.id) {
            return;
        }
        let first = self.detail.borrow().is_none();
        let edits = self.edits.borrow();
        if first || edits.title.is_none() {
            self.show(Shown::Title(detail.title.clone().unwrap_or_default()));
        }
        if first || edits.notes.is_none() {
            self.show(Shown::Notes(detail.notes.clone()));
        }
        drop(edits);
        self.show(Shown::When(when_text(
            detail.record.created_ms,
            detail.length_ms,
        )));
        self.show(Shown::Stages(stage_rows(&detail.record.stages)));
        self.show(Shown::Failures(failures_text(&detail.record.stages)));
        let lines = detail.summary.as_ref();
        self.show(Shown::Summary(
            lines
                .map(|summary| summary_lines(summary, &detail.rows))
                .unwrap_or_default(),
        ));
        let empty = if lines.is_some() { "" } else { NO_SUMMARY };
        self.show(Shown::SummaryEmpty(empty.to_owned()));
        self.show(Shown::SummaryPartial(
            lines.is_some_and(|summary| summary.partial),
        ));
        self.show(Shown::SummaryStale(lines.is_some() && detail.summary_stale));
        self.show(Shown::Transcript(Rows::All(
            detail.rows.iter().map(transcript_line).collect(),
        )));
        self.show(Shown::Speakers(speaker_rows(&detail)));
        let messages = detail.chat.iter();
        self.show(Shown::Chat(Rows::All(
            messages
                .map(|turn| chat_message(turn, &detail.rows))
                .collect(),
        )));
        *self.detail.borrow_mut() = Some(detail);
        self.show_status();
        self.show_asking();
        let row = self.jump_when_open.replace(-1);
        if row >= 0 {
            // The rows have to exist before they can be jumped to.
            self.show(Shown::Tab(TRANSCRIPT_TAB));
            let weak = self.me.clone();
            self.clock.later(
                ROWS_READY,
                Box::new(move || {
                    if let Some(this) = weak.upgrade() {
                        this.show(Shown::Jump(row));
                    }
                }),
            );
        }
    }

    /// Everything that moves with what the flow is doing.
    fn show_status(&self) {
        let status = self.parts.actions.status();
        self.show(Shown::Recording(status.recording.is_some()));
        let start = if status.recording.is_some() { STOP } else { START };
        self.show(Shown::StartLabel(start.to_owned()));
        let id = self.selected();
        let detail = self.detail.borrow();
        let this_recording = id.is_some() && status.recording == id;
        let step = status
            .work
            .iter()
            .find(|(busy, _)| Some(busy) == id.as_ref());
        let busy = step.is_some();
        let idle = detail.is_some() && !busy && !this_recording;
        self.show(Shown::ThisRecording(this_recording));
        self.show(Shown::Busy(busy));
        self.show(Shown::CanResume(idle && status.recording.is_none()));
        self.show(Shown::CanFinish(
            idle && detail.as_ref().is_some_and(|detail| detail.unfinished),
        ));
        self.show(Shown::CanSummarize(
            idle && detail
                .as_ref()
                .is_some_and(|detail| !detail.rows.is_empty()),
        ));
        self.show(Shown::CanExport(id.is_some() && !this_recording));
        self.show(Shown::CanDelete(id.is_some() && !busy && !this_recording));
        let (text, progress) = match step {
            Some((_, step)) => step_text(*step),
            None => ("", -1.0),
        };
        self.show(Shown::ProgressText(text.to_owned()));
        self.show(Shown::Progress(progress));
        let empty = match &*detail {
            Some(detail) if detail.rows.is_empty() && this_recording => LISTENING,
            Some(detail) if detail.rows.is_empty() && !busy => NO_TRANSCRIPT,
            _ => "",
        };
        self.show(Shown::TranscriptEmpty(empty.to_owned()));
        let [first, second] = suggested_questions(this_recording);
        self.show(Shown::Suggestions(vec![first.into(), second.into()]));
        drop(detail);
        let rows = self.list.borrow().clone();
        for (index, mut row) in rows.into_iter().enumerate() {
            let recording = status.recording.as_deref() == Some(row.id.as_str());
            let busy = status.work.iter().any(|(id, _)| row.id == *id);
            if row.recording != recording || row.busy != busy {
                row.recording = recording;
                row.busy = busy;
                self.change_list(Rows::One(index, row));
            }
        }
    }

    /// Adds a live line. It may be in what the window just read already.
    fn live(&self, id: &str, event: LiveEvent) {
        if self.selected().as_deref() != Some(id) {
            return;
        }
        let mut transcript = Transcript::default();
        match event {
            LiveEvent::Segment(segment) => transcript.segments.push(segment),
            LiveEvent::Gap(gap) => transcript.gaps.push(gap),
            LiveEvent::Failed(problem) => return self.notice(&problem),
        }
        let mut detail = self.detail.borrow_mut();
        let Some(detail) = detail.as_mut() else {
            return;
        };
        for row in transcript.rows(&detail.record.speakers) {
            let same = |known: &TranscriptRow| known.at_ms == row.at_ms && known.text == row.text;
            if detail.rows.iter().rev().take(8).any(same) {
                continue;
            }
            let place = detail
                .rows
                .partition_point(|known| known.at_ms <= row.at_ms);
            self.show(Shown::Transcript(Rows::Insert(
                place,
                transcript_line(&row),
            )));
            detail.rows.insert(place, row);
        }
        self.show(Shown::TranscriptEmpty(String::new()));
    }

    pub fn export(&self) {
        let Some(id) = self.selected() else {
            return;
        };
        self.flush_edits();
        let when = self
            .detail
            .borrow()
            .as_ref()
            .map(|detail| when_text(detail.record.created_ms, detail.length_ms))
            .unwrap_or_default();
        self.send(Task::Export { id, when });
    }

    pub fn delete(&self) {
        let Some(id) = self.selected() else {
            return;
        };
        let status = self.parts.actions.status();
        let busy = status.work.iter().any(|(busy, _)| *busy == id);
        if busy || status.recording.as_ref() == Some(&id) {
            return;
        }
        // What was being typed goes with it.
        *self.edits.borrow_mut() = Edits::default();
        self.save_timer.stop();
        self.send(Task::Delete(id));
    }

    fn summarize(&self, mode: SummaryMode) {
        if let Some(id) = self.selected() {
            self.flush_edits();
            self.parts.actions.summarize(&id, mode);
        }
    }

    pub fn ask(&self, question: &str) {
        let question = question.trim();
        let Some(id) = self.selected() else {
            return;
        };
        if question.is_empty() || self.asking.borrow().is_some() {
            return;
        }
        self.flush_edits();
        let run = self.parts.actions.ask(&id, question);
        *self.asking.borrow_mut() = Some(Asking {
            run,
            id,
            question: question.to_owned(),
            text: String::new(),
        });
        self.show(Shown::ChatError(String::new()));
        self.show_asking();
    }

    /// The answer on its way, if it belongs to the meeting on show.
    fn show_asking(&self) {
        let asking = self.asking.borrow();
        let selected = self.selected();
        let here = asking
            .as_ref()
            .filter(|asking| Some(&asking.id) == selected.as_ref());
        let detail = self.detail.borrow();
        let asked = here.is_some() || detail.as_ref().is_some_and(|open| !open.chat.is_empty());
        self.show(Shown::ChatHint(
            if asked { "" } else { ASK_HINT }.to_owned(),
        ));
        self.show(Shown::ChatRunning(asking.is_some()));
        self.show(Shown::StreamingQuestion(
            here.map_or("", |asking| &asking.question).to_owned(),
        ));
        self.show(Shown::StreamingText(
            here.map_or("", |asking| &asking.text).to_owned(),
        ));
    }

    fn chat_update(&self, run: u64, update: ChatUpdate) {
        let mut asking = self.asking.borrow_mut();
        if asking.as_ref().is_none_or(|asking| asking.run.run != run) {
            return;
        }
        match update {
            ChatUpdate::Token(token) => {
                if let Some(asking) = asking.as_mut() {
                    asking.text.push_str(&token);
                }
                drop(asking);
                self.show_asking();
            }
            ChatUpdate::Done(answer) => {
                let id = asking.take().map(|asking| asking.id);
                drop(asking);
                let here = id == self.selected();
                match answer {
                    Ok(turn) if here => {
                        if let Some(detail) = &mut *self.detail.borrow_mut() {
                            let message = chat_message(&turn, &detail.rows);
                            self.show(Shown::Chat(Rows::Add(message)));
                            detail.chat.push(turn);
                        }
                    }
                    Ok(_) => {}
                    Err(problem) if here => self.show(Shown::ChatError(problem)),
                    Err(problem) => self.notice(&problem),
                }
                self.show_asking();
            }
        }
    }
}

impl Meetings {
    /// Sends what's unsaved to the disk thread and says when that thread has done it all, so a
    /// window opened next, or the app going away, can wait for it.
    pub fn finish_up(&self) -> Receiver<()> {
        self.flush_edits();
        let (done, finished) = mpsc::channel();
        self.send(Task::Report(done));
        finished
    }
}

impl Drop for Meetings {
    fn drop(&mut self) {
        // The disk thread finishes its queue before it goes, so this is safe to leave running.
        self.flush_edits();
        if let Some(asking) = &*self.asking.borrow() {
            asking.run.stop();
        }
    }
}
