//! The Meetings window behind `Shell`, as Swift reaches it, on a rehearsal's flow and real meeting
//! records in a temporary folder. `meetings.rs` holds the window's own logic.
#![cfg(target_os = "macos")]

use std::fs;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use wf_core::pill::PillState;
use wf_meetings::{Channel, LiveLog, Meeting, MeetingStore, Segment, SpeakerId};
use wf_shell::app_windows::AppWindow;
use wf_shell::bridge::{EventSink, Shell, ShellEvent};
use wf_shell::meetings::changes::{MeetingsChange, RowsPlace};
use wf_shell::menus::MenuAction;
use wf_shell::paths::Paths;
use wf_shell::rehearsal::ANSWERED;

const WAIT: Duration = Duration::from_secs(10);
/// Past the 700 ms a title or a note waits for.
const SAVED_BY: Duration = Duration::from_secs(5);

struct Recorder(Mutex<Sender<ShellEvent>>);

impl EventSink for Recorder {
    fn on_event(&self, event: ShellEvent) {
        let _ = self.0.lock().unwrap().send(event);
    }
}

/// Runs a call of `Shell` that has nothing to wait for.
fn now<T>(future: impl Future<Output = T>) -> T {
    use std::task::{Context, Poll, Waker};
    match std::pin::pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("nothing here waits"),
    }
}

fn say(log: &mut LiveLog, at_ms: u64, speaker: Option<u32>, text: &str) {
    log.segment(&Segment {
        channel: Channel::You,
        speaker: speaker.map(SpeakerId),
        start_ms: at_ms,
        end_ms: at_ms + 2_000,
        text: text.to_owned(),
    })
    .unwrap();
}

fn seed(meetings: &Path, title: &str) -> Meeting {
    let store = MeetingStore::new(meetings);
    let mut meeting = store.create().unwrap();
    meeting.set_title(Some(title)).unwrap();
    meeting.begin_part().unwrap();
    meeting.end_part(Duration::from_secs(90)).unwrap();
    let mut log = LiveLog::open(&meeting).unwrap();
    say(&mut log, 5_000, None, "Send the contract today.");
    say(&mut log, 12_000, Some(1), "Yes, by Thursday.");
    drop(log);
    meeting
}

/// What the window would hold.
#[derive(Default)]
struct Book {
    rows: Vec<String>,
    selected: String,
    title: String,
    notes: String,
    transcript: Vec<String>,
    speakers: Vec<i32>,
    chat: Vec<wf_shell::meetings::rows::ChatMessage>,
    running: bool,
    streaming: String,
    notice: String,
    busy: bool,
    batches: Vec<Vec<MeetingsChange>>,
}

impl Book {
    fn take(&mut self, changes: Vec<MeetingsChange>) {
        for change in &changes {
            match change {
                MeetingsChange::Meetings {
                    place: RowsPlace::All,
                    rows,
                } => self.rows = rows.iter().map(|row| row.title.clone()).collect(),
                MeetingsChange::SelectedId { id } => self.selected.clone_from(id),
                MeetingsChange::Title { text } => self.title.clone_from(text),
                MeetingsChange::Notes { text } => self.notes.clone_from(text),
                MeetingsChange::Transcript {
                    place: RowsPlace::All,
                    rows,
                } => self.transcript = rows.iter().map(|row| row.text.clone()).collect(),
                MeetingsChange::Speakers { rows } => {
                    self.speakers = rows.iter().map(|row| row.id).collect();
                }
                MeetingsChange::Chat {
                    place: RowsPlace::All,
                    rows,
                } => self.chat.clone_from(rows),
                MeetingsChange::Chat { rows, .. } => self.chat.extend(rows.iter().cloned()),
                MeetingsChange::ChatRunning { running } => self.running = *running,
                MeetingsChange::StreamingText { text } => self.streaming.clone_from(text),
                MeetingsChange::Notice { text } => self.notice.clone_from(text),
                MeetingsChange::Busy { busy } => self.busy = *busy,
                _ => {}
            }
        }
        self.batches.push(changes);
    }
}

struct Rig {
    folder: TempDir,
    shell: Arc<Shell>,
    events: Receiver<ShellEvent>,
    book: Book,
    /// What came besides the window's changes, in order.
    others: Vec<ShellEvent>,
    /// How many batches of changes had come when the window was told to open.
    changes_before_open: Option<usize>,
}

impl Rig {
    fn new() -> Self {
        let folder = TempDir::new().unwrap();
        let (sender, events) = mpsc::channel();
        let paths = Paths::under(folder.path(), "/nowhere/wf-engine-host".into());
        let sink = Arc::new(Recorder(Mutex::new(sender)));
        let shell = now(Shell::rehearse(paths, sink)).unwrap();
        Self {
            folder,
            shell,
            events,
            book: Book::default(),
            others: Vec::new(),
            changes_before_open: None,
        }
    }

    fn meetings_folder(&self) -> std::path::PathBuf {
        self.folder.path().join("meetings")
    }

    fn seed(&self, title: &str) -> String {
        let id = seed(&self.meetings_folder(), title).record().id.clone();
        // Ids are made from the clock.
        thread::sleep(Duration::from_millis(3));
        id
    }

    fn open(&mut self) {
        now(self.shell.menu_action(MenuAction::OpenMeetings)).unwrap();
    }

    /// Takes in what arrives until `done`.
    fn until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + WAIT;
        while !done(self) {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.events.recv_timeout(left) {
                Ok(ShellEvent::Meetings { changes }) => self.book.take(changes),
                Ok(event) => {
                    if matches!(event, ShellEvent::Open { .. }) {
                        self.changes_before_open = Some(self.book.batches.len());
                    }
                    self.others.push(event);
                }
                Err(_) => panic!("no {what} in time"),
            }
        }
    }

    fn open_with(&mut self, titles: usize) {
        self.open();
        self.until("the list", |rig| rig.book.rows.len() == titles);
    }

    fn pick(&mut self, id: &str) {
        self.shell.select_meeting(id.to_owned(), -1).unwrap();
        self.until("the meeting", |rig| {
            rig.book.selected == id && !rig.book.transcript.is_empty()
        });
    }

    fn close(&self) {
        self.shell.window_closed(AppWindow::Meetings).unwrap();
    }

    fn store(&self) -> MeetingStore {
        MeetingStore::new(self.meetings_folder())
    }
}

fn eventually(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + SAVED_BY;
    while !done() {
        assert!(Instant::now() < deadline, "no {what} in time");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_window_opens_before_what_it_shows_arrives() {
    let mut rig = Rig::new();
    rig.seed("Launch");

    rig.open_with(1);

    assert_eq!(rig.changes_before_open, Some(0));
    assert_eq!(rig.book.rows, ["Launch"]);
}

#[test]
fn asking_again_raises_the_window_and_keeps_what_it_shows() {
    let mut rig = Rig::new();
    let id = rig.seed("Launch");
    rig.open_with(1);
    rig.pick(&id);

    rig.open();
    rig.until("the raise", |rig| {
        rig.others.contains(&ShellEvent::Raise {
            window: AppWindow::Meetings,
        })
    });

    rig.shell.edit_meeting_notes("Still here".to_owned()).unwrap();
    let store = rig.store();
    eventually("the save", || {
        store.open(&id).unwrap().notes().unwrap() == "Still here"
    });
}

#[test]
fn a_meeting_lands_all_in_one_change_so_nothing_shows_half_drawn() {
    let mut rig = Rig::new();
    let id = rig.seed("Launch");
    rig.open_with(1);

    rig.pick(&id);

    let batch = rig
        .book
        .batches
        .iter()
        .find(|batch| {
            batch
                .iter()
                .any(|change| matches!(change, MeetingsChange::Transcript { rows, .. } if !rows.is_empty()))
        })
        .unwrap();
    let has = |wanted: fn(&MeetingsChange) -> bool| batch.iter().any(wanted);
    assert!(has(|change| matches!(change, MeetingsChange::Title { .. })));
    assert!(has(|change| matches!(change, MeetingsChange::Stages { .. })));
    assert!(has(|change| matches!(change, MeetingsChange::Chat { .. })));
    assert_eq!(rig.book.title, "Launch");
    assert_eq!(rig.book.speakers, [1]);
}

#[test]
fn a_title_a_note_and_a_name_reach_the_disk_in_the_order_typed() {
    let mut rig = Rig::new();
    let id = rig.seed("Launch");
    rig.open_with(1);
    rig.pick(&id);

    rig.shell.edit_meeting_title("Launch".to_owned()).unwrap();
    rig.shell.edit_meeting_title("Launch plan".to_owned()).unwrap();
    rig.shell.edit_meeting_notes("One".to_owned()).unwrap();
    rig.shell.edit_meeting_notes("One two".to_owned()).unwrap();
    rig.shell.name_speaker(1, "Giulia".to_owned()).unwrap();

    rig.until("the title in the list", |rig| rig.book.rows == ["Launch plan"]);
    let saved = rig.store().open(&id).unwrap();
    assert_eq!(saved.notes().unwrap(), "One two");
    assert_eq!(saved.record().title.as_deref(), Some("Launch plan"));
    let names = &saved.record().speakers;
    assert_eq!(names.get(&SpeakerId(1)).map(String::as_str), Some("Giulia"));
}

#[test]
fn what_was_being_typed_survives_closing_the_window_and_the_next_one_finds_it() {
    let mut rig = Rig::new();
    let id = rig.seed("Launch");
    rig.open_with(1);
    rig.pick(&id);

    rig.shell.edit_meeting_notes("Last words".to_owned()).unwrap();
    rig.close();
    rig.open();
    rig.book = Book::default();
    rig.until("the list", |rig| rig.book.rows == ["Launch"]);
    rig.pick(&id);

    assert_eq!(rig.book.notes, "Last words");
}

#[test]
fn what_was_being_typed_survives_quitting() {
    let mut rig = Rig::new();
    let id = rig.seed("Launch");
    rig.open_with(1);
    rig.pick(&id);

    rig.shell.edit_meeting_notes("Goodnight".to_owned()).unwrap();
    now(rig.shell.quit()).unwrap();

    assert_eq!(rig.store().open(&id).unwrap().notes().unwrap(), "Goodnight");
}

#[test]
fn with_the_window_closed_nothing_more_arrives_and_calls_go_nowhere() {
    let mut rig = Rig::new();
    let id = rig.seed("Launch");
    rig.open_with(1);
    rig.pick(&id);
    rig.close();
    // The next `Open` waits for the old window, so this is the end of what it had to say.
    rig.open();
    rig.until("the open", |rig| {
        rig.others
            .iter()
            .filter(|event| matches!(event, ShellEvent::Open { .. }))
            .count()
            == 2
    });
    rig.close();
    thread::sleep(Duration::from_millis(50));
    while let Ok(event) = rig.events.try_recv() {
        drop(event);
    }

    rig.shell.delete_meeting().unwrap();
    rig.shell.ask_meeting("Anyone?".to_owned()).unwrap();
    rig.shell.select_meeting(id.clone(), -1).unwrap();

    thread::sleep(Duration::from_millis(100));
    assert!(rig.store().open(&id).is_ok());
    assert!(rig.events.try_recv().is_err(), "a closed window said more");
}

#[test]
fn search_finds_the_words_after_the_pause() {
    let mut rig = Rig::new();
    rig.seed("Launch");
    let other = rig.seed("Other");
    let mut log = LiveLog::open(&rig.store().open(&other).unwrap()).unwrap();
    say(&mut log, 30_000, None, "The word is pineapple.");
    drop(log);
    rig.open_with(2);

    rig.shell.search_meetings("pine".to_owned()).unwrap();
    rig.shell.search_meetings("pineapple".to_owned()).unwrap();

    rig.until("the hit", |rig| rig.book.rows == ["Other"]);

    rig.shell.search_meetings(String::new()).unwrap();
    rig.until("the whole list", |rig| rig.book.rows.len() == 2);
}

#[test]
fn export_and_delete_reach_the_disk() {
    let mut rig = Rig::new();
    let id = rig.seed("Launch");
    rig.open_with(1);
    rig.pick(&id);

    rig.shell.export_meeting().unwrap();
    rig.until("the notice", |rig| rig.book.notice.starts_with("Saved to "));
    let exported = rig.folder.path().join("exports/Launch.md");
    assert!(fs::read_to_string(exported).unwrap().contains("# Launch"));

    rig.shell.delete_meeting().unwrap();
    rig.until("the empty list", |rig| rig.book.rows.is_empty());
    assert!(rig.store().open(&id).is_err());
    assert_eq!(rig.book.selected, "");
}

#[test]
fn an_answer_streams_through_the_whole_way_and_its_times_are_checked() {
    let mut rig = Rig::new();
    let id = rig.seed("Launch");
    rig.open_with(1);
    rig.pick(&id);

    rig.shell.ask_meeting("Who sends it?".to_owned()).unwrap();
    rig.until("the first word", |rig| rig.book.streaming.starts_with("Send"));
    rig.until("the answer", |rig| !rig.book.chat.is_empty() && !rig.book.running);

    let [message] = rig.book.chat.as_slice() else {
        panic!("one answer");
    };
    assert_eq!(message.question, "Who sends it?");
    let chips = &message.blocks[0].chips;
    assert_eq!(chips[0].row, 0, "{ANSWERED}");
    assert_eq!(chips[0].help, "Go to [00:05]");
    assert_eq!(chips[1].row, -1);
    assert_eq!(chips[1].text, "[59:59] ?");
}

#[test]
fn stop_ends_an_answer_where_it_got_to() {
    let mut rig = Rig::new();
    let id = rig.seed("Launch");
    rig.open_with(1);
    rig.pick(&id);

    rig.shell.ask_meeting("Who sends it?".to_owned()).unwrap();
    rig.until("the first word", |rig| rig.book.streaming.starts_with("Send"));
    rig.shell.stop_meeting_chat().unwrap();

    rig.until("the end of it", |rig| !rig.book.running);
    assert_eq!(rig.book.chat[0].note, "Stopped.");
}

#[test]
fn a_toggle_reaches_the_flow() {
    let mut rig = Rig::new();
    let id = rig.seed("Launch");
    rig.open_with(1);
    rig.pick(&id);

    // A rehearsal has nothing to record with, and the flow says so.
    rig.shell.toggle_meeting().unwrap();

    rig.until("the flow's answer", |rig| {
        rig.others.iter().any(|event| {
            matches!(
                event,
                ShellEvent::Pill {
                    state: PillState::Error { .. }
                }
            )
        })
    });
}

#[test]
fn work_on_a_meeting_shows_in_the_open_window_as_the_flow_reports_it() {
    let mut rig = Rig::new();
    let id = rig.seed("Launch");
    rig.open_with(1);
    rig.pick(&id);

    rig.shell.regenerate_summary().unwrap();

    rig.until("the work", |rig| rig.book.busy);
    rig.until("its end", |rig| !rig.book.busy);
}
