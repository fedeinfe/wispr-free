//! The Meetings window with no window, over real meeting records in a temporary folder. What the
//! running app would do is a fake, so nothing records and no engine runs. The labels are the ones
//! the window shows.

#[path = "support/clock.rs"]
mod clock;

use std::cell::{Ref, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use wf_dictation::{ChatRun, ChatUpdate, MeetingNews, MeetingsStatus, SummaryMode, WorkStep};
use wf_meetings::{
    ActionItem, Channel, ChatTurn, Item, LiveEvent, LiveLog, Meeting, MeetingStore, Segment,
    SpeakerId, Stage, StageState, Summary,
};
use wf_shell::meetings::rows::{
    ChatMessage, MeetingRow, SpeakerRow, StageMark, StageRow, SummaryLine, TranscriptLine,
};
use wf_shell::meetings::{MeetingActions, Meetings, MeetingsParts, Shown};

use clock::ManualClock;

const WAIT: Duration = Duration::from_secs(5);
const SUMMARY: i32 = 0;
const TRANSCRIPT: i32 = 2;

#[derive(Default)]
struct FakeActions {
    calls: RefCell<Vec<String>>,
    status: RefCell<MeetingsStatus>,
    runs: RefCell<Vec<Arc<AtomicBool>>>,
}

impl FakeActions {
    fn called(&self, call: &str) -> bool {
        self.calls.borrow().iter().any(|known| known == call)
    }
}

impl MeetingActions for FakeActions {
    fn toggle(&self) {
        self.calls.borrow_mut().push("toggle".to_owned());
    }

    fn resume(&self, id: &str) {
        self.calls.borrow_mut().push(format!("resume {id}"));
    }

    fn finish(&self, id: &str) {
        self.calls.borrow_mut().push(format!("finish {id}"));
    }

    fn summarize(&self, id: &str, mode: SummaryMode) {
        self.calls
            .borrow_mut()
            .push(format!("summarize {id} {mode:?}"));
    }

    fn cancel(&self, id: &str) {
        self.calls.borrow_mut().push(format!("cancel {id}"));
    }

    fn ask(&self, id: &str, question: &str) -> ChatRun {
        self.calls.borrow_mut().push(format!("ask {id} {question}"));
        let cancel = Arc::<AtomicBool>::default();
        let mut runs = self.runs.borrow_mut();
        runs.push(Arc::clone(&cancel));
        ChatRun::new(runs.len() as u64, cancel)
    }

    fn status(&self) -> MeetingsStatus {
        self.status.borrow().clone()
    }
}

fn say(log: &mut LiveLog, at_ms: u64, channel: Channel, speaker: Option<u32>, text: &str) {
    log.segment(&Segment {
        channel,
        speaker: speaker.map(SpeakerId),
        start_ms: at_ms,
        end_ms: at_ms + 2_000,
        text: text.to_owned(),
    })
    .unwrap();
}

/// A finished meeting: two voices, a summary, some notes.
fn seed(store: &MeetingStore, title: &str) -> Meeting {
    let mut meeting = store.create().unwrap();
    meeting.begin_part().unwrap();
    meeting.end_part(Duration::from_secs(90)).unwrap();
    let mut log = LiveLog::open(&meeting).unwrap();
    say(
        &mut log,
        5_000,
        Channel::You,
        None,
        "Can you send the contract?",
    );
    say(
        &mut log,
        12_000,
        Channel::Others,
        Some(1),
        "Yes, by Thursday.",
    );
    say(
        &mut log,
        75_000,
        Channel::Others,
        Some(2),
        "The budget is approved, says finance.",
    );
    drop(log);
    meeting.set_notes("Call the supplier").unwrap();
    let hash = wf_meetings::transcript_hash(&meeting.transcript().unwrap());
    meeting
        .save_summary(&Summary {
            title: Some(title.to_owned()),
            tldr: "The contract goes out by Thursday.".to_owned(),
            key_points: vec![Item {
                text: "The budget is approved".to_owned(),
                at_ms: Some(75_000),
            }],
            decisions: vec![Item {
                text: "A time that is no line".to_owned(),
                at_ms: Some(33_000),
            }],
            action_items: vec![ActionItem {
                task: "Send the contract".to_owned(),
                owner: Some("Speaker 1".to_owned()),
                due: Some("Thursday".to_owned()),
                at_ms: Some(12_000),
            }],
            partial: false,
            transcript_hash: hash,
        })
        .unwrap();
    for stage in [Stage::Diarization, Stage::Refine, Stage::Summary] {
        meeting
            .set_stage(stage, StageState::Done { at_ms: 1 })
            .unwrap();
    }
    // Ids are made from the clock.
    thread::sleep(Duration::from_millis(3));
    meeting
}

/// What the window shows, as it would hold it.
#[derive(Default)]
struct Seen {
    meetings: Vec<MeetingRow>,
    list_hint: String,
    detail_hint: String,
    start_label: String,
    can_export: bool,
    can_delete: bool,
    chat_hint: String,
    recording: bool,
    selected_id: String,
    title: String,
    notes: String,
    this_recording: bool,
    busy: bool,
    can_finish: bool,
    can_resume: bool,
    can_summarize: bool,
    stages: Vec<StageRow>,
    progress_text: String,
    progress: f32,
    notice: String,
    tab: i32,
    jump_row: i32,
    summary: Vec<SummaryLine>,
    summary_stale: bool,
    transcript: Vec<TranscriptLine>,
    transcript_empty: String,
    speakers: Vec<SpeakerRow>,
    chat: Vec<ChatMessage>,
    suggestions: Vec<String>,
    chat_running: bool,
    streaming_question: String,
    streaming_text: String,
    chat_error: String,
}

impl Seen {
    fn take(&mut self, shown: Shown) {
        match shown {
            Shown::Meetings(change) => change.apply(&mut self.meetings),
            Shown::ListHint(hint) => self.list_hint = hint,
            Shown::Recording(recording) => self.recording = recording,
            Shown::SelectedId(id) => self.selected_id = id,
            Shown::Title(title) => self.title = title,
            Shown::Notes(notes) => self.notes = notes,
            Shown::ThisRecording(recording) => self.this_recording = recording,
            Shown::Busy(busy) => self.busy = busy,
            Shown::CanFinish(can) => self.can_finish = can,
            Shown::CanResume(can) => self.can_resume = can,
            Shown::CanSummarize(can) => self.can_summarize = can,
            Shown::Stages(rows) => self.stages = rows,
            Shown::ProgressText(text) => self.progress_text = text,
            Shown::Progress(progress) => self.progress = progress,
            Shown::Notice(text) => self.notice = text,
            Shown::Tab(tab) => self.tab = tab,
            Shown::Jump(row) => {
                self.tab = TRANSCRIPT;
                self.jump_row = row;
            }
            Shown::Summary(lines) => self.summary = lines,
            Shown::SummaryStale(stale) => self.summary_stale = stale,
            Shown::Transcript(change) => change.apply(&mut self.transcript),
            Shown::TranscriptEmpty(text) => self.transcript_empty = text,
            Shown::Speakers(rows) => self.speakers = rows,
            Shown::Chat(change) => change.apply(&mut self.chat),
            Shown::Suggestions(questions) => self.suggestions = questions,
            Shown::ChatRunning(running) => self.chat_running = running,
            Shown::StreamingQuestion(question) => self.streaming_question = question,
            Shown::StreamingText(text) => self.streaming_text = text,
            Shown::ChatError(error) => self.chat_error = error,
            Shown::DetailHint(hint) => self.detail_hint = hint,
            Shown::StartLabel(label) => self.start_label = label,
            Shown::CanExport(can) => self.can_export = can,
            Shown::CanDelete(can) => self.can_delete = can,
            Shown::ChatHint(hint) => self.chat_hint = hint,
            Shown::When(_)
            | Shown::Failures(_)
            | Shown::SummaryEmpty(_)
            | Shown::SummaryPartial(_) => {}
        }
    }
}

struct Harness {
    dir: tempfile::TempDir,
    store: MeetingStore,
    actions: Rc<FakeActions>,
    clock: Rc<ManualClock>,
    seen: Rc<RefCell<Seen>>,
    meetings: Option<Rc<Meetings>>,
}

impl Harness {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path().join("meetings"));
        Self {
            dir,
            store,
            actions: Rc::default(),
            clock: Rc::default(),
            seen: Rc::default(),
            meetings: None,
        }
    }

    fn parts(&self) -> MeetingsParts {
        MeetingsParts {
            store: self.store.clone(),
            actions: Rc::clone(&self.actions) as Rc<dyn MeetingActions>,
            export_dir: self.export_dir(),
            bring_to_front: Rc::new(|| {}),
        }
    }

    fn export_dir(&self) -> PathBuf {
        self.dir.path().join("exports")
    }

    fn open(mut self) -> Self {
        std::fs::create_dir_all(self.export_dir()).unwrap();
        let seen = Rc::clone(&self.seen);
        let view = Rc::new(move |shown| seen.borrow_mut().take(shown));
        let meetings = Meetings::new(self.parts(), view, self.clock.clone(), Arc::new(|| {}));
        self.meetings = Some(meetings.unwrap());
        self
    }

    fn meetings(&self) -> &Rc<Meetings> {
        self.meetings.as_ref().unwrap()
    }

    fn ui(&self) -> Ref<'_, Seen> {
        self.seen.borrow()
    }

    /// Lets the disk thread answer until `done`.
    fn wait(&self, what: &str, done: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + WAIT;
        loop {
            self.meetings().drain_events();
            if done(self) {
                return;
            }
            assert!(Instant::now() < deadline, "no {what} in time");
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn settle(&self) {
        thread::sleep(Duration::from_millis(30));
        self.meetings().drain_events();
    }

    fn wait_listed(&self, count: usize) {
        self.wait("list", |this| this.ui().meetings.len() == count);
    }

    fn pick(&self, index: usize) {
        let id = self.ui().meetings[index].id.clone();
        self.meetings().select(&id, -1);
        self.wait("meeting", |this| {
            this.ui().selected_id == id && !this.ui().transcript.is_empty()
        });
    }

    fn click(&self, label: &str) {
        let meetings = self.meetings();
        match label {
            "Delete" => meetings.delete(),
            "Export" => meetings.export(),
            "Regenerate" => meetings.regenerate(),
            "Update summary" => meetings.update_summary(),
            "Cancel" => meetings.cancel_work(),
            "Finish processing" => meetings.finish(),
            "Start meeting notes" => meetings.toggle_meeting(),
            "Keep recording" => meetings.resume(),
            "Stop" => meetings.stop_chat(),
            _ => panic!("no control labelled {label}"),
        }
    }

    fn lines(&self) -> Vec<String> {
        let lines = &self.ui().transcript;
        lines.iter().map(|line| line.text.clone()).collect()
    }
}

fn ready(titles: &[&str]) -> (Harness, Vec<String>) {
    let harness = Harness::new();
    let ids = titles
        .iter()
        .map(|title| seed(&harness.store, title).record().id.clone())
        .collect();
    let harness = harness.open();
    harness.wait_listed(titles.len());
    (harness, ids)
}

#[test]
fn the_list_shows_every_meeting_newest_first() {
    let (harness, _) = ready(&["Older", "Newer"]);

    let rows = harness.ui().meetings.clone();
    assert_eq!(rows[0].title, "Newer");
    assert_eq!(rows[1].title, "Older");
    assert!(rows[0].when.ends_with("· 2 min"));
    assert_eq!(harness.ui().selected_id, "");
}

#[test]
fn an_empty_library_says_how_to_begin() {
    let harness = Harness::new().open();
    harness.settle();
    assert!(harness.ui().list_hint.contains("Start one"));
}

#[test]
fn opening_a_meeting_fills_all_four_tabs() {
    let (harness, ids) = ready(&["Launch"]);

    harness.pick(0);

    let ui = harness.ui();
    assert_eq!(ui.selected_id, ids[0].as_str());
    assert_eq!(ui.title, "Launch");
    assert_eq!(ui.notes, "Call the supplier");
    assert_eq!(harness.lines().len(), 3);
    let summary: Vec<String> = ui.summary.iter().map(|line| line.text.clone()).collect();
    assert!(summary.contains(&"The budget is approved".to_owned()));
    assert!(summary.contains(&"Speaker 1: Send the contract (Thursday)".to_owned()));
    assert_eq!(ui.speakers.len(), 2);
}

#[test]
fn a_summary_time_on_a_real_line_jumps_and_a_wrong_one_does_not() {
    let (harness, _) = ready(&["Launch"]);
    harness.pick(0);

    let ui = harness.ui();
    let line = |text: &str| ui.summary.iter().find(|line| line.text == text).unwrap();
    let wrong = line("A time that is no line");
    assert_eq!((wrong.time.as_str(), wrong.row), ("[00:33]", -1));
    let right = line("The budget is approved");
    assert_eq!((right.time.as_str(), right.row), ("[01:15]", 2));
    assert_eq!(ui.tab, SUMMARY);
}

#[test]
fn search_finds_words_in_the_transcript_and_opens_at_the_line() {
    let (harness, _) = ready(&["One", "Two"]);

    harness.meetings().search("finance".to_owned());
    harness.clock.advance(Duration::from_millis(200));
    harness.wait("hits", |this| {
        this.ui()
            .meetings
            .first()
            .is_some_and(|row| !row.snippet.is_empty())
    });
    let hit = harness.ui().meetings[0].clone();
    assert!(hit.snippet.contains("finance"), "{}", hit.snippet);
    assert_eq!(hit.hit_row, 2);

    harness.meetings().select(&hit.id, hit.hit_row);
    harness.wait("the transcript", |this| this.ui().tab == TRANSCRIPT);
    harness.clock.advance(Duration::from_millis(100));
    assert_eq!(harness.ui().jump_row, 2);
}

#[test]
fn a_search_that_finds_nothing_says_so_and_clearing_it_brings_the_list_back() {
    let (harness, _) = ready(&["One", "Two"]);

    harness.meetings().search("zebra".to_owned());
    harness.clock.advance(Duration::from_millis(200));
    harness.wait_listed(0);
    assert!(harness.ui().list_hint.contains("Nothing"));

    harness.meetings().search("".to_owned());
    harness.clock.advance(Duration::from_millis(200));
    harness.wait_listed(2);
}

#[test]
fn a_new_title_goes_to_disk_and_to_the_list() {
    let (harness, ids) = ready(&["Launch"]);
    harness.pick(0);

    harness
        .meetings()
        .title_edited("The launch plan".to_owned());
    harness.clock.advance(Duration::from_secs(1));

    harness.wait("the new title", |this| {
        this.ui().meetings[0].title == "The launch plan"
    });
    let saved = harness.store.open(&ids[0]).unwrap();
    assert_eq!(saved.record().title.as_deref(), Some("The launch plan"));
}

#[test]
fn notes_are_saved_after_a_pause() {
    let (harness, ids) = ready(&["Launch"]);
    harness.pick(0);

    harness.meetings().notes_edited("New notes".to_owned());
    harness.settle();
    assert_eq!(
        harness.store.open(&ids[0]).unwrap().notes().unwrap(),
        "Call the supplier"
    );

    harness.clock.advance(Duration::from_secs(1));
    harness.wait("the save", |this| {
        this.store.open(&ids[0]).unwrap().notes().unwrap() == "New notes"
    });
}

#[test]
fn notes_being_typed_survive_switching_meetings() {
    let (harness, ids) = ready(&["One", "Two"]);
    harness.pick(1);

    harness.meetings().notes_edited("Typed just now".to_owned());
    harness.pick(0);

    harness.wait("the save", |this| {
        this.store.open(&ids[0]).unwrap().notes().unwrap() == "Typed just now"
    });
    assert_eq!(harness.ui().notes, "Call the supplier");
}

#[test]
fn notes_being_typed_survive_closing_the_window() {
    let (mut harness, ids) = ready(&["Launch"]);
    harness.pick(0);
    harness.meetings().notes_edited("Last words".to_owned());

    harness.meetings = None;

    let store = harness.store.clone();
    let deadline = Instant::now() + WAIT;
    while store.open(&ids[0]).unwrap().notes().unwrap() != "Last words" {
        assert!(Instant::now() < deadline, "the notes were lost");
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn a_named_speaker_is_named_all_through_the_meeting() {
    let (harness, ids) = ready(&["Launch"]);
    harness.pick(0);

    harness.meetings().speaker_edited(1, "Giulia".to_owned());
    harness.clock.advance(Duration::from_secs(1));

    harness.wait("the name", |this| {
        this.ui()
            .transcript
            .iter()
            .any(|line| line.speaker == "Giulia")
    });
    let summary = harness.ui().summary.clone();
    assert!(
        summary
            .iter()
            .any(|line| line.text.starts_with("Giulia: Send"))
    );
    let saved = harness.store.open(&ids[0]).unwrap();
    let names = &saved.record().speakers;
    assert_eq!(names.get(&SpeakerId(1)).map(String::as_str), Some("Giulia"));
}

#[test]
fn delete_removes_the_meeting_and_clears_the_selection() {
    let (harness, ids) = ready(&["Launch"]);
    harness.pick(0);

    harness.click("Delete");

    harness.wait_listed(0);
    assert!(harness.store.open(&ids[0]).is_err());
    assert_eq!(harness.ui().selected_id, "");
}

#[test]
fn a_meeting_being_worked_on_cannot_be_deleted() {
    let (harness, ids) = ready(&["Launch"]);
    harness.pick(0);

    harness.actions.status.borrow_mut().work = vec![(ids[0].clone(), WorkStep::Waiting)];
    harness.meetings().news(MeetingNews::Working {
        id: ids[0].clone(),
        step: WorkStep::Waiting,
    });

    assert!(harness.ui().busy);
    harness.meetings().delete();
    harness.settle();
    assert!(harness.store.open(&ids[0]).is_ok());
}

#[test]
fn export_writes_markdown_and_says_where() {
    let (harness, _) = ready(&["Launch"]);
    harness.pick(0);

    harness.click("Export");

    harness.wait("the export", |this| !this.ui().notice.is_empty());
    let notice = harness.ui().notice.clone();
    assert!(notice.starts_with("Saved to "), "{notice}");
    let path = Path::new(notice.trim_start_matches("Saved to "));
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains("# Launch"));
    assert!(text.contains("The budget is approved"));
}

#[test]
fn regenerate_and_update_reach_the_flow() {
    let (harness, ids) = ready(&["Launch"]);
    harness.pick(0);
    assert!(!harness.ui().summary_stale);

    harness.click("Regenerate");
    assert!(
        harness
            .actions
            .called(&format!("summarize {} Regenerate", ids[0]))
    );

    let mut log = LiveLog::open(&harness.store.open(&ids[0]).unwrap()).unwrap();
    say(&mut log, 80_000, Channel::You, None, "One more thing.");
    drop(log);
    harness.meetings().news(MeetingNews::WorkDone {
        id: ids[0].clone(),
        problem: None,
    });
    harness.wait("the stale mark", |this| this.ui().summary_stale);
    harness.click("Update summary");
    assert!(
        harness
            .actions
            .called(&format!("summarize {} Update", ids[0]))
    );
}

#[test]
fn stages_show_progress_and_say_what_failed() {
    let (harness, ids) = ready(&["Launch"]);
    let mut meeting = harness.store.open(&ids[0]).unwrap();
    let failed = StageState::Failed {
        reason: "the speaker model is damaged".to_owned(),
    };
    meeting.set_stage(Stage::Diarization, failed).unwrap();
    harness.pick(0);

    let stages = harness.ui().stages.clone();
    assert_eq!(stages[0].state, StageMark::Failed);
    assert_eq!(stages[0].reason, "the speaker model is damaged");
    assert!(!harness.ui().meetings.is_empty());

    let step = WorkStep::Refining {
        done_ms: 500,
        total_ms: 2_000,
    };
    harness.actions.status.borrow_mut().work = vec![(ids[0].clone(), step)];
    harness.meetings().news(MeetingNews::Working {
        id: ids[0].clone(),
        step,
    });
    assert_eq!(harness.ui().progress, 0.25);
    assert_eq!(
        harness.ui().progress_text,
        "Going over the transcript again"
    );

    harness.click("Cancel");
    assert!(harness.actions.called(&format!("cancel {}", ids[0])));
}

#[test]
fn a_problem_with_the_work_is_shown() {
    let (harness, ids) = ready(&["Launch"]);
    harness.pick(0);
    harness.meetings().news(MeetingNews::WorkDone {
        id: ids[0].clone(),
        problem: Some("the disk is full".to_owned()),
    });
    assert_eq!(harness.ui().notice, "the disk is full");
}

#[test]
fn an_unfinished_meeting_offers_to_finish() {
    let (harness, ids) = ready(&["Launch"]);
    let mut meeting = harness.store.open(&ids[0]).unwrap();
    meeting
        .set_stage(Stage::Summary, StageState::Pending)
        .unwrap();
    harness.pick(0);

    harness.click("Finish processing");

    assert!(harness.actions.called(&format!("finish {}", ids[0])));
}

#[test]
fn the_start_button_toggles_and_keep_recording_resumes() {
    let (harness, ids) = ready(&["Launch"]);
    harness.pick(0);

    harness.click("Start meeting notes");
    assert!(harness.actions.called("toggle"));
    harness.click("Keep recording");
    assert!(harness.actions.called(&format!("resume {}", ids[0])));
}

#[test]
fn a_meeting_started_with_nothing_open_comes_up_by_itself_and_grows() {
    let harness = Harness::new().open();
    harness.settle();
    let mut meeting = harness.store.create().unwrap();
    meeting.begin_part().unwrap();
    let id = meeting.record().id.clone();
    harness.actions.status.borrow_mut().recording = Some(id.clone());

    harness.meetings().news(MeetingNews::Recording(id.clone()));
    harness.wait("the meeting", |this| {
        this.ui().meetings.len() == 1 && this.ui().this_recording
    });
    harness.wait("the hint", |this| {
        this.ui().transcript_empty.contains("Listening")
    });
    assert!(harness.ui().recording);
    assert!(harness.ui().meetings[0].recording);

    let line = |at_ms, text: &str| MeetingNews::Live {
        id: id.clone(),
        event: LiveEvent::Segment(Segment {
            channel: Channel::Others,
            speaker: None,
            start_ms: at_ms,
            end_ms: at_ms + 1_000,
            text: text.to_owned(),
        }),
    };
    harness.meetings().news(line(6_000, "Later line"));
    harness.meetings().news(line(2_000, "Earlier line"));
    harness.meetings().news(line(2_000, "Earlier line"));

    assert_eq!(harness.lines(), ["Earlier line", "Later line"]);
    assert_eq!(harness.ui().transcript_empty, "");
}

#[test]
fn a_live_line_of_another_meeting_is_not_shown_here() {
    let (harness, _) = ready(&["Launch"]);
    harness.pick(0);
    harness.meetings().news(MeetingNews::Live {
        id: "another".to_owned(),
        event: LiveEvent::Failed("no".to_owned()),
    });
    assert_eq!(harness.ui().notice, "");
}

fn turn(question: &str, answer: &str) -> ChatTurn {
    ChatTurn {
        question: question.to_owned(),
        answer: answer.to_owned(),
        asked_ms: 0,
        stopped: false,
        partial_transcript: false,
    }
}

#[test]
fn a_question_streams_and_ends_with_times_that_jump() {
    let (harness, ids) = ready(&["Launch"]);
    harness.pick(0);

    harness.meetings().ask("Who sends it?");
    assert!(
        harness
            .actions
            .called(&format!("ask {} Who sends it?", ids[0]))
    );
    assert!(harness.ui().chat_running);
    assert_eq!(harness.ui().streaming_question, "Who sends it?");

    let update = |update| MeetingNews::Chat { run: 1, update };
    harness
        .meetings()
        .news(update(ChatUpdate::Token("Speaker 1".to_owned())));
    harness
        .meetings()
        .news(update(ChatUpdate::Token(" sends it".to_owned())));
    assert_eq!(harness.ui().streaming_text, "Speaker 1 sends it");

    harness.meetings().news(update(ChatUpdate::Done(Ok(turn(
        "Who sends it?",
        "By Thursday, yes. [00:12] [05:00]",
    )))));

    assert!(!harness.ui().chat_running);
    assert_eq!(harness.ui().streaming_question, "");
    let messages = harness.ui().chat.clone();
    let block = &messages[0].blocks[0];
    assert_eq!(block.text, "By Thursday, yes.");
    assert_eq!(block.chips[0].row, 1, "a time on a line");
    assert_eq!(block.chips[1].row, -1, "[05:00] matches no line");
}

#[test]
fn only_one_answer_runs_and_stop_stops_it() {
    let (harness, _) = ready(&["Launch"]);
    harness.pick(0);

    harness.meetings().ask("First?");
    harness.meetings().ask("Second?");
    assert_eq!(harness.actions.runs.borrow().len(), 1);

    harness.click("Stop");
    assert!(harness.actions.runs.borrow()[0].load(Ordering::SeqCst));

    let stopped = ChatTurn {
        stopped: true,
        ..turn("First?", "Part of")
    };
    harness.meetings().news(MeetingNews::Chat {
        run: 1,
        update: ChatUpdate::Done(Ok(stopped)),
    });
    assert_eq!(harness.ui().chat[0].note, "Stopped.");
    assert!(!harness.ui().chat_running);
}

#[test]
fn suggested_prompts_ask_and_change_during_a_recording() {
    let (harness, ids) = ready(&["Launch"]);
    harness.pick(0);
    let after: Vec<String> = harness.ui().suggestions.clone();
    assert!(!after.is_empty());

    harness.meetings().ask(after[0].as_str());
    assert!(
        harness
            .actions
            .called(&format!("ask {} {}", ids[0], after[0]))
    );

    harness.actions.status.borrow_mut().recording = Some(ids[0].clone());
    harness
        .meetings()
        .news(MeetingNews::Recording(ids[0].clone()));
    let during: Vec<String> = harness.ui().suggestions.clone();
    assert_ne!(after, during);
}

#[test]
fn a_failed_answer_is_shown_and_the_chat_works_again() {
    let (harness, _) = ready(&["Launch"]);
    harness.pick(0);
    harness.meetings().ask("First?");

    harness.meetings().news(MeetingNews::Chat {
        run: 1,
        update: ChatUpdate::Done(Err("The chat needs a model.".to_owned())),
    });

    assert_eq!(harness.ui().chat_error, "The chat needs a model.");
    assert!(!harness.ui().chat_running);
    harness.meetings().ask("Again?");
    assert_eq!(harness.ui().chat_error, "");
}

#[test]
fn an_answer_for_another_meeting_waits_for_it_and_does_not_show_here() {
    let (harness, _) = ready(&["One", "Two"]);
    harness.pick(0);
    harness.meetings().ask("First?");
    harness.pick(1);

    assert_eq!(harness.ui().streaming_question, "");
    assert!(harness.ui().chat_running);
    harness.meetings().news(MeetingNews::Chat {
        run: 1,
        update: ChatUpdate::Done(Ok(turn("First?", "Yes."))),
    });
    assert_eq!(harness.ui().chat.len(), 0);
}

#[test]
fn saved_answers_are_checked_again_when_the_meeting_opens() {
    let (harness, ids) = ready(&["Launch"]);
    let mut meeting = harness.store.open(&ids[0]).unwrap();
    meeting
        .add_chat_turn(turn("Who?", "By Thursday. [00:12]"))
        .unwrap();
    meeting
        .add_chat_turn(turn("And?", "No one. [07:07]"))
        .unwrap();

    harness.pick(0);

    let messages = harness.ui().chat.clone();
    assert_eq!(messages[0].blocks[0].chips[0].row, 1);
    assert_eq!(messages[1].blocks[0].chips[0].row, -1);
    assert!(messages[1].note.contains("No time in this answer"));
}

#[test]
fn closing_the_window_stops_its_answer() {
    let (mut harness, _) = ready(&["Launch"]);
    harness.pick(0);
    harness.meetings().ask("First?");

    harness.meetings = None;

    assert!(harness.actions.runs.borrow()[0].load(Ordering::SeqCst));
}

#[test]
fn with_nothing_picked_the_window_says_what_to_do() {
    let (harness, _) = ready(&["Launch"]);
    assert!(harness.ui().detail_hint.starts_with("Pick a meeting"));
    assert_eq!(harness.ui().start_label, "Start meeting notes");
    assert!(!harness.ui().can_export && !harness.ui().can_delete);

    let empty = Harness::new().open();
    empty.settle();
    assert_eq!(empty.ui().detail_hint, "");
}

#[test]
fn a_recording_meeting_can_be_neither_exported_nor_deleted_and_the_button_says_stop() {
    let (harness, ids) = ready(&["Launch"]);
    harness.pick(0);
    assert!(harness.ui().can_export && harness.ui().can_delete);

    harness.actions.status.borrow_mut().recording = Some(ids[0].clone());
    harness
        .meetings()
        .news(MeetingNews::Recording(ids[0].clone()));

    assert_eq!(harness.ui().start_label, "Stop meeting notes");
    assert!(!harness.ui().can_export && !harness.ui().can_delete);
}

#[test]
fn a_meeting_at_work_can_be_exported_but_not_deleted() {
    let (harness, ids) = ready(&["Launch"]);
    harness.pick(0);

    harness.actions.status.borrow_mut().work = vec![(ids[0].clone(), WorkStep::Waiting)];
    harness.meetings().news(MeetingNews::Working {
        id: ids[0].clone(),
        step: WorkStep::Waiting,
    });

    assert!(harness.ui().can_export && !harness.ui().can_delete);
}

#[test]
fn the_chat_explains_itself_until_something_is_asked() {
    let (harness, _) = ready(&["Launch"]);
    harness.pick(0);
    assert!(harness.ui().chat_hint.starts_with("Answers come only"));

    harness.meetings().ask("First?");
    assert_eq!(harness.ui().chat_hint, "");

    harness.meetings().news(MeetingNews::Chat {
        run: 1,
        update: ChatUpdate::Done(Ok(turn("First?", "Yes."))),
    });
    assert_eq!(harness.ui().chat_hint, "");
}

#[test]
fn finishing_up_says_when_the_disk_has_it_all() {
    let (harness, ids) = ready(&["Launch"]);
    harness.pick(0);
    harness.meetings().notes_edited("Last words".to_owned());

    harness.meetings().finish_up().recv_timeout(WAIT).unwrap();

    let saved = harness.store.open(&ids[0]).unwrap();
    assert_eq!(saved.notes().unwrap(), "Last words");
}

#[test]
fn a_window_of_a_thousand_lines_opens_without_trouble() {
    let harness = Harness::new();
    let mut meeting = harness.store.create().unwrap();
    meeting.begin_part().unwrap();
    meeting.end_part(Duration::from_secs(3_000)).unwrap();
    let mut log = LiveLog::open(&meeting).unwrap();
    for line in 0..1_000 {
        let text = format!("Line number {line} of a long meeting.");
        say(&mut log, line * 3_000, Channel::You, None, &text);
    }
    drop(log);
    let harness = harness.open();
    harness.wait_listed(1);
    harness.pick(0);
    assert_eq!(harness.lines().len(), 1_000);
}
