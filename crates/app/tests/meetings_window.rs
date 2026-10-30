//! The Meetings window driven through its real .slint UI on Slint's testing backend, over real
//! meeting records in a temporary folder. What the running app would do is a fake, so nothing
//! records and no engine runs.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use i_slint_backend_testing::{
    AccessibleRole, ElementHandle, init_no_event_loop, mock_elapsed_time,
};
use slint::{ComponentHandle, LogicalSize, Model};
use wf_dictation::{ChatRun, ChatUpdate, MeetingNews, MeetingsStatus, SummaryMode, WorkStep};
use wf_meetings::{
    ActionItem, Channel, ChatTurn, Item, LiveEvent, LiveLog, Meeting, MeetingStore, Segment,
    SpeakerId, Stage, StageState, Summary,
};
use wispr_free::meetings_window::{
    MeetingActions, MeetingsBoard, MeetingsController, MeetingsHost, MeetingsParts,
};
use wispr_free::{MeetingsUi, MeetingsWindow};

const WAIT: Duration = Duration::from_secs(5);
const SUMMARY: i32 = 0;
const NOTES: i32 = 1;
const TRANSCRIPT: i32 = 2;
const CHAT: i32 = 3;

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

struct Harness {
    dir: tempfile::TempDir,
    store: MeetingStore,
    actions: Rc<FakeActions>,
    controller: Option<MeetingsController>,
}

impl Harness {
    fn new() -> Self {
        init_no_event_loop();
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path().join("meetings"));
        Self {
            dir,
            store,
            actions: Rc::default(),
            controller: None,
        }
    }

    fn parts(&self) -> MeetingsParts {
        MeetingsParts {
            store: self.store.clone(),
            actions: Rc::clone(&self.actions) as Rc<dyn MeetingActions>,
            theme: 0,
            export_dir: self.export_dir(),
            bring_to_front: Rc::new(|| {}),
        }
    }

    fn export_dir(&self) -> PathBuf {
        self.dir.path().join("exports")
    }

    fn open(mut self) -> Self {
        std::fs::create_dir_all(self.export_dir()).unwrap();
        let controller = MeetingsController::new(self.parts()).unwrap();
        let window = controller.window();
        window.window().set_size(LogicalSize::new(980.0, 680.0));
        window.show().unwrap();
        self.controller = Some(controller);
        self
    }

    fn controller(&self) -> &MeetingsController {
        self.controller.as_ref().unwrap()
    }

    fn window(&self) -> &MeetingsWindow {
        self.controller().window()
    }

    fn ui(&self) -> MeetingsUi<'_> {
        self.window().global::<MeetingsUi>()
    }

    /// Lets the disk thread answer until `done`.
    fn wait(&self, what: &str, done: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + WAIT;
        loop {
            self.controller().drain_events();
            if done(self) {
                return;
            }
            assert!(Instant::now() < deadline, "no {what} in time");
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn settle(&self) {
        thread::sleep(Duration::from_millis(30));
        self.controller().drain_events();
    }

    fn wait_listed(&self, count: usize) {
        self.wait("list", |this| this.ui().get_meetings().row_count() == count);
    }

    fn pick(&self, index: usize) {
        let id = self.ui().get_meetings().row_data(index).unwrap().id;
        self.ui().invoke_select(id.clone(), -1);
        self.wait("meeting", |this| {
            this.ui().get_selected_id() == id && this.ui().get_transcript().row_count() > 0
        });
    }

    fn all(&self, label: &str) -> Vec<ElementHandle> {
        ElementHandle::find_by_accessible_label(self.window(), label).collect()
    }

    fn click(&self, label: &str) {
        // A text can carry the same label as a control.
        self.all(label)
            .into_iter()
            .find(|element| element.accessible_role() != Some(AccessibleRole::Text))
            .unwrap_or_else(|| panic!("no control labelled {label}"))
            .invoke_accessible_default_action();
    }

    fn on_screen(&self, label: &str) -> bool {
        !self.all(label).is_empty()
    }

    fn lines(&self) -> Vec<String> {
        let lines = self.ui().get_transcript();
        lines.iter().map(|line| line.text.to_string()).collect()
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

    let rows = harness.ui().get_meetings();
    assert_eq!(rows.row_data(0).unwrap().title, "Newer");
    assert_eq!(rows.row_data(1).unwrap().title, "Older");
    assert!(rows.row_data(0).unwrap().when.ends_with("· 2 min"));
    assert_eq!(harness.ui().get_selected_id(), "");
}

#[test]
fn an_empty_library_says_how_to_begin() {
    let harness = Harness::new().open();
    harness.settle();
    assert!(harness.ui().get_list_hint().contains("Start one"));
}

#[test]
fn opening_a_meeting_fills_all_four_tabs() {
    let (harness, ids) = ready(&["Launch"]);

    harness.pick(0);

    let ui = harness.ui();
    assert_eq!(ui.get_selected_id(), ids[0].as_str());
    assert_eq!(ui.get_title(), "Launch");
    assert_eq!(ui.get_notes(), "Call the supplier");
    assert_eq!(harness.lines().len(), 3);
    let summary: Vec<String> = ui
        .get_summary()
        .iter()
        .map(|line| line.text.into())
        .collect();
    assert!(summary.contains(&"The budget is approved".to_owned()));
    assert!(summary.contains(&"Speaker 1: Send the contract (Thursday)".to_owned()));
    assert_eq!(ui.get_speakers().row_count(), 2);
}

#[test]
fn the_summary_warns_it_can_be_wrong_and_offers_the_transcript() {
    let (harness, _) = ready(&["Launch"]);
    harness.pick(0);
    assert!(harness.on_screen(
        "Written by the model, so it can miss or misread things. The transcript is one click away."
    ));
    assert!(harness.on_screen("Transcript"));
}

#[test]
fn a_summary_time_on_a_real_line_jumps_and_a_wrong_one_does_not() {
    let (harness, _) = ready(&["Launch"]);
    harness.pick(0);

    assert!(harness.on_screen("[00:33] matches no line of the transcript"));
    assert_eq!(harness.ui().get_tab(), SUMMARY);
    harness.click("Go to [01:15]");

    assert_eq!(harness.ui().get_tab(), TRANSCRIPT);
    assert_eq!(harness.ui().get_jump_row(), 2);
}

#[test]
fn search_finds_words_in_the_transcript_and_opens_at_the_line() {
    let (harness, _) = ready(&["One", "Two"]);

    harness.ui().set_query("finance".into());
    harness.ui().invoke_search();
    mock_elapsed_time(Duration::from_millis(200));
    harness.wait("hits", |this| {
        this.ui()
            .get_meetings()
            .row_data(0)
            .is_some_and(|row| !row.snippet.is_empty())
    });
    let hit = harness.ui().get_meetings().row_data(0).unwrap();
    assert!(hit.snippet.contains("finance"), "{}", hit.snippet);
    assert_eq!(hit.hit_row, 2);

    harness.ui().invoke_select(hit.id, hit.hit_row);
    harness.wait("the transcript", |this| this.ui().get_tab() == TRANSCRIPT);
    mock_elapsed_time(Duration::from_millis(100));
    assert_eq!(harness.ui().get_jump_row(), 2);
}

#[test]
fn a_search_that_finds_nothing_says_so_and_clearing_it_brings_the_list_back() {
    let (harness, _) = ready(&["One", "Two"]);

    harness.ui().set_query("zebra".into());
    harness.ui().invoke_search();
    mock_elapsed_time(Duration::from_millis(200));
    harness.wait_listed(0);
    assert!(harness.ui().get_list_hint().contains("Nothing"));

    harness.ui().set_query("".into());
    harness.ui().invoke_search();
    mock_elapsed_time(Duration::from_millis(200));
    harness.wait_listed(2);
}

#[test]
fn a_new_title_goes_to_disk_and_to_the_list() {
    let (harness, ids) = ready(&["Launch"]);
    harness.pick(0);

    harness.ui().set_title("The launch plan".into());
    harness.ui().invoke_title_edited();
    mock_elapsed_time(Duration::from_secs(1));

    harness.wait("the new title", |this| {
        this.ui().get_meetings().row_data(0).unwrap().title == "The launch plan"
    });
    let saved = harness.store.open(&ids[0]).unwrap();
    assert_eq!(saved.record().title.as_deref(), Some("The launch plan"));
}

#[test]
fn notes_are_saved_after_a_pause() {
    let (harness, ids) = ready(&["Launch"]);
    harness.pick(0);

    harness.ui().set_notes("New notes".into());
    harness.ui().invoke_notes_edited();
    harness.settle();
    assert_eq!(
        harness.store.open(&ids[0]).unwrap().notes().unwrap(),
        "Call the supplier"
    );

    mock_elapsed_time(Duration::from_secs(1));
    harness.wait("the save", |this| {
        this.store.open(&ids[0]).unwrap().notes().unwrap() == "New notes"
    });
}

#[test]
fn notes_being_typed_survive_switching_meetings() {
    let (harness, ids) = ready(&["One", "Two"]);
    harness.pick(1);

    harness.ui().set_notes("Typed just now".into());
    harness.ui().invoke_notes_edited();
    harness.pick(0);

    harness.wait("the save", |this| {
        this.store.open(&ids[0]).unwrap().notes().unwrap() == "Typed just now"
    });
    assert_eq!(harness.ui().get_notes(), "Call the supplier");
}

#[test]
fn notes_being_typed_survive_closing_the_window() {
    let (mut harness, ids) = ready(&["Launch"]);
    harness.pick(0);
    harness.ui().set_notes("Last words".into());
    harness.ui().invoke_notes_edited();

    harness.controller = None;

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

    harness.ui().invoke_speaker_edited(1, "Giulia".into());
    mock_elapsed_time(Duration::from_secs(1));

    harness.wait("the name", |this| {
        this.ui()
            .get_transcript()
            .iter()
            .any(|line| line.speaker == "Giulia")
    });
    let summary = harness.ui().get_summary();
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
fn delete_asks_first_and_then_it_is_gone() {
    let (harness, ids) = ready(&["Launch"]);
    harness.pick(0);

    harness.click("Delete");
    assert!(harness.store.open(&ids[0]).is_ok());
    harness.click("Keep it");
    assert!(harness.on_screen("Delete"));

    harness.click("Delete");
    harness.click("Delete for good");

    harness.wait_listed(0);
    assert!(harness.store.open(&ids[0]).is_err());
    assert_eq!(harness.ui().get_selected_id(), "");
}

#[test]
fn a_meeting_being_worked_on_cannot_be_deleted() {
    let (harness, ids) = ready(&["Launch"]);
    harness.pick(0);

    harness.actions.status.borrow_mut().work = vec![(ids[0].clone(), WorkStep::Waiting)];
    harness.controller().news(MeetingNews::Working {
        id: ids[0].clone(),
        step: WorkStep::Waiting,
    });

    assert!(!harness.on_screen("Delete"));
    harness.ui().invoke_delete();
    harness.settle();
    assert!(harness.store.open(&ids[0]).is_ok());
}

#[test]
fn export_writes_markdown_and_says_where() {
    let (harness, _) = ready(&["Launch"]);
    harness.pick(0);

    harness.click("Export");

    harness.wait("the export", |this| !this.ui().get_notice().is_empty());
    let notice = harness.ui().get_notice().to_string();
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
    assert!(!harness.on_screen("Update summary"), "not stale");

    harness.click("Regenerate");
    assert!(
        harness
            .actions
            .called(&format!("summarize {} Regenerate", ids[0]))
    );

    let mut log = LiveLog::open(&harness.store.open(&ids[0]).unwrap()).unwrap();
    say(&mut log, 80_000, Channel::You, None, "One more thing.");
    drop(log);
    harness.controller().news(MeetingNews::WorkDone {
        id: ids[0].clone(),
        problem: None,
    });
    harness.wait("the stale mark", |this| this.ui().get_summary_stale());
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

    let stages = harness.ui().get_stages();
    assert_eq!(stages.row_data(0).unwrap().state, 3);
    assert_eq!(
        stages.row_data(0).unwrap().reason,
        "the speaker model is damaged"
    );
    assert!(harness.ui().get_meetings().row_data(0).is_some());

    let step = WorkStep::Refining {
        done_ms: 500,
        total_ms: 2_000,
    };
    harness.actions.status.borrow_mut().work = vec![(ids[0].clone(), step)];
    harness.controller().news(MeetingNews::Working {
        id: ids[0].clone(),
        step,
    });
    assert_eq!(harness.ui().get_progress(), 0.25);
    assert_eq!(
        harness.ui().get_progress_text(),
        "Going over the transcript again"
    );

    harness.click("Cancel");
    assert!(harness.actions.called(&format!("cancel {}", ids[0])));
}

#[test]
fn a_problem_with_the_work_is_shown() {
    let (harness, ids) = ready(&["Launch"]);
    harness.pick(0);
    harness.controller().news(MeetingNews::WorkDone {
        id: ids[0].clone(),
        problem: Some("the disk is full".to_owned()),
    });
    assert_eq!(harness.ui().get_notice(), "the disk is full");
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

    harness
        .controller()
        .news(MeetingNews::Recording(id.clone()));
    harness.wait("the meeting", |this| {
        this.ui().get_meetings().row_count() == 1 && this.ui().get_this_recording()
    });
    harness.wait("the hint", |this| {
        this.ui().get_transcript_empty().contains("Listening")
    });
    assert!(harness.ui().get_recording());
    assert!(harness.ui().get_meetings().row_data(0).unwrap().recording);

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
    harness.controller().news(line(6_000, "Later line"));
    harness.controller().news(line(2_000, "Earlier line"));
    harness.controller().news(line(2_000, "Earlier line"));

    assert_eq!(harness.lines(), ["Earlier line", "Later line"]);
    assert_eq!(harness.ui().get_transcript_empty(), "");
}

#[test]
fn a_live_line_of_another_meeting_is_not_shown_here() {
    let (harness, _) = ready(&["Launch"]);
    harness.pick(0);
    harness.controller().news(MeetingNews::Live {
        id: "another".to_owned(),
        event: LiveEvent::Failed("no".to_owned()),
    });
    assert_eq!(harness.ui().get_notice(), "");
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
    harness.ui().set_tab(CHAT);

    harness.ui().invoke_ask("Who sends it?".into());
    assert!(
        harness
            .actions
            .called(&format!("ask {} Who sends it?", ids[0]))
    );
    assert!(harness.ui().get_chat_running());
    assert_eq!(harness.ui().get_streaming_question(), "Who sends it?");

    let update = |update| MeetingNews::Chat { run: 1, update };
    harness
        .controller()
        .news(update(ChatUpdate::Token("Speaker 1".to_owned())));
    harness
        .controller()
        .news(update(ChatUpdate::Token(" sends it".to_owned())));
    assert_eq!(harness.ui().get_streaming_text(), "Speaker 1 sends it");

    harness.controller().news(update(ChatUpdate::Done(Ok(turn(
        "Who sends it?",
        "By Thursday, yes. [00:12] [05:00]",
    )))));

    assert!(!harness.ui().get_chat_running());
    assert_eq!(harness.ui().get_streaming_question(), "");
    let messages = harness.ui().get_chat();
    let block = messages.row_data(0).unwrap().blocks.row_data(0).unwrap();
    assert_eq!(block.text, "By Thursday, yes.");
    assert!(harness.on_screen("[05:00] matches no line of the transcript"));

    harness.click("Go to [00:12]");
    assert_eq!(harness.ui().get_tab(), TRANSCRIPT);
    assert_eq!(harness.ui().get_jump_row(), 1);
}

#[test]
fn only_one_answer_runs_and_stop_stops_it() {
    let (harness, _) = ready(&["Launch"]);
    harness.pick(0);
    harness.ui().set_tab(CHAT);

    harness.ui().invoke_ask("First?".into());
    harness.ui().invoke_ask("Second?".into());
    assert_eq!(harness.actions.runs.borrow().len(), 1);

    harness.click("Stop");
    assert!(harness.actions.runs.borrow()[0].load(Ordering::SeqCst));

    let stopped = ChatTurn {
        stopped: true,
        ..turn("First?", "Part of")
    };
    harness.controller().news(MeetingNews::Chat {
        run: 1,
        update: ChatUpdate::Done(Ok(stopped)),
    });
    assert_eq!(
        harness.ui().get_chat().row_data(0).unwrap().note,
        "Stopped."
    );
    assert!(harness.on_screen("Send"));
}

#[test]
fn suggested_prompts_ask_and_change_during_a_recording() {
    let (harness, ids) = ready(&["Launch"]);
    harness.pick(0);
    harness.ui().set_tab(CHAT);
    let after: Vec<String> = harness
        .ui()
        .get_suggestions()
        .iter()
        .map(Into::into)
        .collect();
    assert!(!after.is_empty());

    harness.click(&after[0]);
    assert!(
        harness
            .actions
            .called(&format!("ask {} {}", ids[0], after[0]))
    );

    harness.actions.status.borrow_mut().recording = Some(ids[0].clone());
    harness
        .controller()
        .news(MeetingNews::Recording(ids[0].clone()));
    let during: Vec<String> = harness
        .ui()
        .get_suggestions()
        .iter()
        .map(Into::into)
        .collect();
    assert_ne!(after, during);
}

#[test]
fn a_failed_answer_is_shown_and_the_chat_works_again() {
    let (harness, _) = ready(&["Launch"]);
    harness.pick(0);
    harness.ui().set_tab(CHAT);
    harness.ui().invoke_ask("First?".into());

    harness.controller().news(MeetingNews::Chat {
        run: 1,
        update: ChatUpdate::Done(Err("The chat needs a model.".to_owned())),
    });

    assert_eq!(harness.ui().get_chat_error(), "The chat needs a model.");
    assert!(!harness.ui().get_chat_running());
    harness.ui().invoke_ask("Again?".into());
    assert_eq!(harness.ui().get_chat_error(), "");
}

#[test]
fn an_answer_for_another_meeting_waits_for_it_and_does_not_show_here() {
    let (harness, _) = ready(&["One", "Two"]);
    harness.pick(0);
    harness.ui().invoke_ask("First?".into());
    harness.pick(1);

    assert_eq!(harness.ui().get_streaming_question(), "");
    assert!(harness.ui().get_chat_running());
    harness.controller().news(MeetingNews::Chat {
        run: 1,
        update: ChatUpdate::Done(Ok(turn("First?", "Yes."))),
    });
    assert_eq!(harness.ui().get_chat().row_count(), 0);
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
    harness.ui().set_tab(CHAT);

    assert!(harness.on_screen("Go to [00:12]"));
    assert!(harness.on_screen("[07:07] matches no line of the transcript"));
    let messages = harness.ui().get_chat();
    assert!(
        messages
            .row_data(1)
            .unwrap()
            .note
            .contains("No time in this answer")
    );
}

#[test]
fn closing_the_window_stops_its_answer() {
    let (mut harness, _) = ready(&["Launch"]);
    harness.pick(0);
    harness.ui().invoke_ask("First?".into());

    harness.controller = None;

    assert!(harness.actions.runs.borrow()[0].load(Ordering::SeqCst));
}

#[test]
fn the_tabs_switch_panels() {
    let (harness, _) = ready(&["Launch"]);
    harness.pick(0);
    for (label, tab) in [
        ("Notes", NOTES),
        ("Transcript", TRANSCRIPT),
        ("Chat", CHAT),
        ("Summary", SUMMARY),
    ] {
        harness.click(label);
        assert_eq!(harness.ui().get_tab(), tab);
    }
}

#[test]
fn the_host_keeps_nothing_while_closed_and_one_window_while_open() {
    init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let opens = Rc::new(Cell::new(0));
    let fronts = Rc::new(Cell::new(0));
    let parts = {
        let store = MeetingStore::new(dir.path().join("meetings"));
        let (opens, fronts) = (Rc::clone(&opens), Rc::clone(&fronts));
        move || {
            opens.set(opens.get() + 1);
            let fronts = Rc::clone(&fronts);
            MeetingsParts {
                store: store.clone(),
                actions: Rc::new(FakeActions::default()) as Rc<dyn MeetingActions>,
                theme: 0,
                export_dir: PathBuf::new(),
                bring_to_front: Rc::new(move || fronts.set(fronts.get() + 1)),
            }
        }
    };
    let board = MeetingsBoard::default();
    let host = MeetingsHost::new(Rc::new(parts), &board);

    board.post(MeetingNews::ListChanged);
    assert!(!host.is_open());

    host.open().unwrap();
    host.open().unwrap();
    assert_eq!(opens.get(), 1);
    assert_eq!(fronts.get(), 2);
    let window = host.window().unwrap();
    board.post(MeetingNews::ListChanged);

    host.close();
    assert!(!host.is_open());
    assert!(
        window.upgrade().is_none(),
        "the window is gone with the host's hold on it"
    );
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
    harness.ui().invoke_jump(999);
    assert_eq!(harness.ui().get_jump_row(), 999);
}
