//! The Meetings window (spec §4): created on open, dropped on close. The meeting data and what's
//! done with it live in `wf_meetings` and `wf_dictation`. This only binds them to the .slint, with
//! disk work on [`disk`]'s thread and engine work in the flow, so the UI thread never waits.

mod disk;
mod host;
pub mod rows;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, PlatformError, SharedString, Timer, TimerMode};
use slint::{VecModel, Weak};
use wf_dictation::{ChatRun, ChatUpdate, Dictation, MeetingNews, MeetingsStatus, SummaryMode};
use wf_meetings::{
    LiveEvent, MeetingDetail, MeetingEntry, MeetingStore, SearchHit, SpeakerId, Transcript,
    TranscriptRow, suggested_questions,
};

use crate::{
    ChatMessage, MeetingRow, MeetingsUi, MeetingsWindow, Prefs, SpeakerRow, StageRow, SummaryLine,
    TranscriptLine,
};
use disk::Task;
use rows::{
    chat_message, failures_text, meeting_row, speaker_rows, stage_rows, step_text, summary_lines,
    transcript_line, when_text,
};

pub use host::{MakeParts, MeetingsBoard, MeetingsHost};

/// How long after the last key an edit goes to disk.
const SAVE_AFTER: Duration = Duration::from_millis(700);
/// How long after the last key a search runs.
const SEARCH_AFTER: Duration = Duration::from_millis(150);
/// Enough for the frame that makes a just-opened meeting's rows.
const ROWS_READY: Duration = Duration::from_millis(50);
const TRANSCRIPT_TAB: i32 = 2;

const EMPTY_LIST: &str = "No meetings yet. Start one here, from the tray, or with the shortcut.";
const NO_HITS: &str = "Nothing matches that.";
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

/// Delivers [`Event`]s to the window from any thread. Dropped silently once the window is gone.
#[derive(Clone)]
pub struct Mailbox {
    events: Sender<Event>,
    window: Weak<MeetingsWindow>,
}

impl Mailbox {
    pub fn post(&self, event: Event) {
        if self.events.send(event).is_ok() {
            let _ = self.window.upgrade_in_event_loop(|window| {
                window.global::<MeetingsUi>().invoke_events_arrived();
            });
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
pub struct MeetingsController {
    inner: Rc<Inner>,
}

struct Inner {
    window: MeetingsWindow,
    parts: MeetingsParts,
    tasks: Sender<Task>,
    events: Receiver<Event>,
    list: Rc<VecModel<MeetingRow>>,
    entries: RefCell<Vec<MeetingEntry>>,
    /// What the search box last found. `None` shows every meeting.
    hits: RefCell<Option<Vec<SearchHit>>>,
    detail: RefCell<Option<MeetingDetail>>,
    /// The line to show once the meeting being opened is there.
    jump_when_open: Cell<i32>,
    summary: Rc<VecModel<SummaryLine>>,
    transcript: Rc<VecModel<TranscriptLine>>,
    speakers: Rc<VecModel<SpeakerRow>>,
    stages: Rc<VecModel<StageRow>>,
    chat: Rc<VecModel<ChatMessage>>,
    asking: RefCell<Option<Asking>>,
    edits: RefCell<Edits>,
    save_timer: Timer,
    search_timer: Timer,
}

/// Connects a `MeetingsUi` callback to a method of [`Inner`] through a weak reference.
macro_rules! wire {
    ($inner:ident, $ui:ident . $on:ident, |$this:ident $(, $arg:ident)*| $body:expr) => {{
        let weak = Rc::downgrade($inner);
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
        let (sender, events) = mpsc::channel();
        let mailbox = Mailbox {
            events: sender,
            window: window.as_weak(),
        };
        let (tasks, queue) = mpsc::channel();
        let (store, export_dir) = (parts.store.clone(), parts.export_dir.clone());
        thread::Builder::new()
            .name("wf-meetings-disk".into())
            .spawn(move || disk::run(&store, &export_dir, queue, &mailbox))
            .map_err(|error| PlatformError::Other(error.to_string()))?;
        let inner = Rc::new(Inner {
            window,
            parts,
            tasks,
            events,
            list: Rc::default(),
            entries: RefCell::default(),
            hits: RefCell::default(),
            detail: RefCell::default(),
            jump_when_open: Cell::new(-1),
            summary: Rc::default(),
            transcript: Rc::default(),
            speakers: Rc::default(),
            stages: Rc::default(),
            chat: Rc::default(),
            asking: RefCell::default(),
            edits: RefCell::default(),
            save_timer: Timer::default(),
            search_timer: Timer::default(),
        });
        inner.connect();
        inner.show_status();
        inner.send(Task::List);
        Ok(Self { inner })
    }

    pub fn window(&self) -> &MeetingsWindow {
        &self.inner.window
    }

    pub fn bring_to_front(&self) {
        (self.inner.parts.bring_to_front)();
    }

    /// Takes in what the flow reported. On the UI thread.
    pub fn news(&self, news: MeetingNews) {
        self.inner.news(news);
    }

    /// Runs what the disk thread has reported so far.
    pub fn drain_events(&self) {
        self.inner.drain_events();
    }
}

impl Drop for MeetingsController {
    fn drop(&mut self) {
        // The disk thread finishes its queue before it goes, so this is safe to leave running.
        self.inner.flush_edits();
        if let Some(asking) = &*self.inner.asking.borrow() {
            asking.run.stop();
        }
    }
}

impl Inner {
    fn ui(&self) -> MeetingsUi<'_> {
        self.window.global::<MeetingsUi>()
    }

    fn send(&self, task: Task) {
        let _ = self.tasks.send(task);
    }

    fn selected(&self) -> Option<String> {
        let id = self.ui().get_selected_id();
        (!id.is_empty()).then(|| id.to_string())
    }

    fn connect(self: &Rc<Self>) {
        let inner = self;
        let ui = self.ui();
        let prefs = self.window.global::<Prefs>();
        let mut values = prefs.get_values();
        values.theme = self.parts.theme;
        prefs.set_values(values);

        ui.set_meetings(ModelRc::from(Rc::clone(&self.list)));
        ui.set_summary(ModelRc::from(Rc::clone(&self.summary)));
        ui.set_transcript(ModelRc::from(Rc::clone(&self.transcript)));
        ui.set_speakers(ModelRc::from(Rc::clone(&self.speakers)));
        ui.set_stages(ModelRc::from(Rc::clone(&self.stages)));
        ui.set_chat(ModelRc::from(Rc::clone(&self.chat)));

        wire!(inner, ui.on_events_arrived, |this| this.drain_events());
        wire!(inner, ui.on_toggle_meeting, |this| this
            .parts
            .actions
            .toggle());
        wire!(inner, ui.on_search, |this| Inner::search_soon(&this));
        wire!(inner, ui.on_select, |this, id, row| this.select(&id, row));
        wire!(inner, ui.on_title_edited, |this| {
            if let Some(id) = this.selected() {
                let title = this.ui().get_title().to_string();
                this.edits.borrow_mut().title = Some((id, title));
                Inner::save_soon(&this);
            }
        });
        wire!(inner, ui.on_notes_edited, |this| {
            if let Some(id) = this.selected() {
                let notes = this.ui().get_notes().to_string();
                this.edits.borrow_mut().notes = Some((id, notes));
                Inner::save_soon(&this);
            }
        });
        wire!(inner, ui.on_speaker_edited, |this, speaker, name| {
            if let Some(id) = this.selected() {
                let speaker = SpeakerId(speaker as u32);
                let mut edits = this.edits.borrow_mut();
                edits.speakers.retain(|(_, known, _)| *known != speaker);
                edits.speakers.push((id, speaker, name.to_string()));
                drop(edits);
                Inner::save_soon(&this);
            }
        });
        wire!(inner, ui.on_export, |this| this.export());
        wire!(inner, ui.on_delete, |this| this.delete());
        wire!(inner, ui.on_resume, |this| {
            if let Some(id) = this.selected() {
                this.flush_edits();
                this.parts.actions.resume(&id);
            }
        });
        wire!(inner, ui.on_finish, |this| {
            if let Some(id) = this.selected() {
                this.parts.actions.finish(&id);
            }
        });
        wire!(inner, ui.on_cancel_work, |this| {
            if let Some(id) = this.selected() {
                this.parts.actions.cancel(&id);
            }
        });
        wire!(inner, ui.on_regenerate, |this| this
            .summarize(SummaryMode::Regenerate));
        wire!(inner, ui.on_update_summary, |this| this
            .summarize(SummaryMode::Update));
        wire!(inner, ui.on_ask, |this, question| this.ask(&question));
        wire!(inner, ui.on_stop_chat, |this| {
            if let Some(asking) = &*this.asking.borrow() {
                asking.run.stop();
            }
        });
    }

    fn drain_events(&self) {
        while let Ok(event) = self.events.try_recv() {
            self.handle(event);
        }
    }

    fn handle(&self, event: Event) {
        match event {
            Event::Listed(entries) => {
                *self.entries.borrow_mut() = entries;
                self.show_list();
                let query = self.ui().get_query();
                if !query.trim().is_empty() {
                    self.send(Task::Search(query.to_string()));
                }
            }
            Event::Found { query, hits } => {
                if self.ui().get_query() == query {
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
        self.ui().set_notice(text.into());
    }

    fn news(&self, news: MeetingNews) {
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

    fn search_soon(this: &Rc<Self>) {
        let weak = Rc::downgrade(this);
        this.search_timer
            .start(TimerMode::SingleShot, SEARCH_AFTER, move || {
                if let Some(this) = weak.upgrade() {
                    let query = this.ui().get_query();
                    if query.trim().is_empty() {
                        this.hits.borrow_mut().take();
                        this.show_list();
                    } else {
                        this.send(Task::Search(query.to_string()));
                    }
                }
            });
    }

    fn save_soon(this: &Rc<Self>) {
        let weak = Rc::downgrade(this);
        this.save_timer
            .start(TimerMode::SingleShot, SAVE_AFTER, move || {
                if let Some(this) = weak.upgrade() {
                    this.flush_edits();
                }
            });
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
        self.ui().set_list_hint(hint.into());
        self.list.set_vec(rows);
    }

    fn select(&self, id: &str, row: i32) {
        self.flush_edits();
        let ui = self.ui();
        if self.selected().as_deref() != Some(id) {
            ui.set_selected_id(id.into());
            ui.set_notice(SharedString::new());
            self.detail.borrow_mut().take();
            self.transcript.set_vec(Vec::new());
            self.show_status();
        }
        self.jump_when_open.set(row);
        self.send(Task::Open(id.to_owned()));
    }

    fn clear_selection(&self) {
        self.ui().set_selected_id(SharedString::new());
        self.detail.borrow_mut().take();
        self.show_status();
    }

    fn show_detail(&self, detail: MeetingDetail) {
        if self.selected().as_ref() != Some(&detail.record.id) {
            return;
        }
        let ui = self.ui();
        let first = self.detail.borrow().is_none();
        let edits = self.edits.borrow();
        if first || edits.title.is_none() {
            ui.set_title(detail.title.as_deref().unwrap_or_default().into());
        }
        if first || edits.notes.is_none() {
            ui.set_notes(detail.notes.as_str().into());
        }
        drop(edits);
        ui.set_when(when_text(detail.record.created_ms, detail.length_ms).into());
        self.stages.set_vec(stage_rows(&detail.record.stages));
        ui.set_failures(failures_text(&detail.record.stages).into());
        let lines = detail.summary.as_ref();
        self.summary.set_vec(
            lines
                .map(|summary| summary_lines(summary, &detail.rows))
                .unwrap_or_default(),
        );
        ui.set_summary_empty(if lines.is_some() { "" } else { NO_SUMMARY }.into());
        ui.set_summary_partial(lines.is_some_and(|summary| summary.partial));
        ui.set_summary_stale(lines.is_some() && detail.summary_stale);
        self.transcript
            .set_vec(detail.rows.iter().map(transcript_line).collect::<Vec<_>>());
        self.speakers.set_vec(speaker_rows(&detail));
        let messages = detail.chat.iter();
        self.chat.set_vec(
            messages
                .map(|turn| chat_message(turn, &detail.rows))
                .collect::<Vec<_>>(),
        );
        *self.detail.borrow_mut() = Some(detail);
        self.show_status();
        self.show_asking();
        let row = self.jump_when_open.replace(-1);
        if row >= 0 {
            // The rows have to exist before they can be jumped to.
            self.ui().set_tab(TRANSCRIPT_TAB);
            let window = self.window.as_weak();
            Timer::single_shot(ROWS_READY, move || {
                if let Some(window) = window.upgrade() {
                    window.global::<MeetingsUi>().invoke_jump(row);
                }
            });
        }
    }

    /// Everything that moves with what the flow is doing.
    fn show_status(&self) {
        let status = self.parts.actions.status();
        let ui = self.ui();
        ui.set_recording(status.recording.is_some());
        let id = self.selected();
        let detail = self.detail.borrow();
        let this_recording = id.is_some() && status.recording == id;
        let step = status
            .work
            .iter()
            .find(|(busy, _)| Some(busy) == id.as_ref());
        let busy = step.is_some();
        let idle = detail.is_some() && !busy && !this_recording;
        ui.set_this_recording(this_recording);
        ui.set_busy(busy);
        ui.set_can_resume(idle && status.recording.is_none());
        ui.set_can_finish(idle && detail.as_ref().is_some_and(|detail| detail.unfinished));
        ui.set_can_summarize(
            idle && detail
                .as_ref()
                .is_some_and(|detail| !detail.rows.is_empty()),
        );
        let (text, progress) = match step {
            Some((_, step)) => step_text(*step),
            None => ("", -1.0),
        };
        ui.set_progress_text(text.into());
        ui.set_progress(progress);
        ui.set_transcript_empty(
            match &*detail {
                Some(detail) if detail.rows.is_empty() && this_recording => LISTENING,
                Some(detail) if detail.rows.is_empty() && !busy => NO_TRANSCRIPT,
                _ => "",
            }
            .into(),
        );
        let [first, second] = suggested_questions(this_recording);
        ui.set_suggestions(ModelRc::new(VecModel::from(vec![
            SharedString::from(first),
            SharedString::from(second),
        ])));
        drop(detail);
        self.list.iter().enumerate().for_each(|(index, mut row)| {
            let recording = status.recording.as_deref() == Some(row.id.as_str());
            let busy = status.work.iter().any(|(id, _)| row.id == *id);
            if row.recording != recording || row.busy != busy {
                row.recording = recording;
                row.busy = busy;
                self.list.set_row_data(index, row);
            }
        });
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
            self.transcript.insert(place, transcript_line(&row));
            detail.rows.insert(place, row);
        }
        self.ui().set_transcript_empty(SharedString::new());
    }

    fn export(&self) {
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

    fn delete(&self) {
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

    fn ask(&self, question: &str) {
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
        self.ui().set_chat_error(SharedString::new());
        self.show_asking();
    }

    /// The answer on its way, if it belongs to the meeting on show.
    fn show_asking(&self) {
        let asking = self.asking.borrow();
        let selected = self.selected();
        let here = asking
            .as_ref()
            .filter(|asking| Some(&asking.id) == selected.as_ref());
        let ui = self.ui();
        ui.set_chat_running(asking.is_some());
        ui.set_streaming_question(here.map_or("", |asking| &asking.question).into());
        ui.set_streaming_text(here.map_or("", |asking| &asking.text).into());
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
                            self.chat.push(chat_message(&turn, &detail.rows));
                            detail.chat.push(turn);
                        }
                    }
                    Ok(_) => {}
                    Err(problem) if here => self.ui().set_chat_error(problem.into()),
                    Err(problem) => self.notice(&problem),
                }
                self.show_asking();
            }
        }
    }
}
