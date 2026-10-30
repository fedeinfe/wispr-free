//! The dictation flow (spec §3): shortcut → capture → VAD → ASR → rule cleanup → LLM cleanup →
//! local passes → insertion, and everything that keeps it honest: lazy models, cancel, errors the
//! user can see. It has no UI. [`Dictation`] takes input and reports through [`Event`]s, and every
//! contact with the outside world is a seam in [`Parts`], so tests run the whole flow on fakes.

mod coordinator;
mod corrections;
mod engines;
mod media;
mod meeting;
mod meeting_work;
pub mod plan;
pub mod probe;
pub mod remote;
mod session;
mod worker;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use wf_audio::{Capture, CaptureConfig, CaptureEvent};
use wf_core::dictation::{DictationMachine, PillInput};
use wf_core::pill::PillState;
use wf_core::settings::Settings;
use wf_models::Catalog;
use wf_platform::context::{ContextSource, SystemContext};
use wf_platform::insert::{InsertError, InsertOutcome, SystemInserter};
use wf_platform::keys::KeySignal;
use wf_platform::media::{MediaControl, SystemMedia};

pub use corrections::{LOOK_EVERY, STEADY_LOOKS, WATCH_FOR};
pub use engines::Engines;
pub use meeting::{
    ChatUpdate, MeetingNews, MeetingRoom, MeetingTape, MeetingsStatus, SystemMeetingRoom,
};
pub use meeting_work::{ChatRun, WorkStep};
pub use wf_meetings::SummaryMode;

use coordinator::{Coordinator, Input};
use meeting::Meetings;

/// An insertion holds the history's thread for about half a second.
const FORGET_LIMIT: Duration = Duration::from_secs(2);

/// How the flow tells time, as a monotonic `Duration` from any origin. Only deadlines go through
/// it (the state machine's, showing an error, unloading models), so tests can jump past them.
pub trait Clock: Send + Sync {
    fn now(&self) -> Duration;
}

/// The real clock.
pub struct SystemClock(Instant);

impl Default for SystemClock {
    fn default() -> Self {
        Self(Instant::now())
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

/// An open mic. Dropping it closes the device, which turns the OS's mic indicator off.
pub trait MicStream: Send {
    fn events(&self) -> &Receiver<CaptureEvent>;
}

impl MicStream for Capture {
    fn events(&self) -> &Receiver<CaptureEvent> {
        Capture::events(self)
    }
}

pub trait Microphone: Send + Sync {
    /// Why the mic can't work, if that is known before trying.
    fn problem(&self) -> Option<String>;
    /// Opens the best-ranked device. Called once per dictation, on key-down.
    fn open(&self, ranking: Vec<String>) -> Result<Box<dyn MicStream>, String>;
}

/// The computer's microphones.
pub struct SystemMicrophone;

impl Microphone for SystemMicrophone {
    fn problem(&self) -> Option<String> {
        wf_platform::permissions::check()
            .microphone_problem()
            .map(str::to_owned)
    }

    fn open(&self, ranking: Vec<String>) -> Result<Box<dyn MicStream>, String> {
        let config = CaptureConfig {
            ranking,
            ..CaptureConfig::default()
        };
        match Capture::start(config) {
            Ok(capture) => Ok(Box::new(capture)),
            Err(error) => Err(error.to_string()),
        }
    }
}

/// Puts text where the user is working. Runs on its own thread, one job at a time.
pub trait Insertion: Send {
    /// Blocks until the clipboard is back. `on_pasted` runs right after the paste keystroke.
    fn insert(
        &mut self,
        text: &str,
        on_pasted: &mut dyn FnMut(),
    ) -> Result<InsertOutcome, InsertError>;
    fn press_enter(&mut self) -> Result<(), InsertError>;
    /// Leaves `text` on the clipboard.
    fn copy(&mut self, text: &str) -> Result<(), InsertError>;
}

impl Insertion for SystemInserter {
    fn insert(
        &mut self,
        text: &str,
        on_pasted: &mut dyn FnMut(),
    ) -> Result<InsertOutcome, InsertError> {
        self.insert_then(text, on_pasted)
    }

    fn press_enter(&mut self) -> Result<(), InsertError> {
        SystemInserter::press_enter(self)
    }

    fn copy(&mut self, text: &str) -> Result<(), InsertError> {
        SystemInserter::copy(self, text)
    }
}

/// What the rest of the app gets to see. Delivered on the flow's threads, so a UI must hop to its
/// own before touching anything.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// What the pill should show. Hidden and Ready are the pill's own business.
    Pill(PillState),
    /// Mic level, 0.0..=1.0, about every 150 ms while recording.
    Level(f32),
    /// A short message for the pill, such as "Copied — paste with Ctrl+Cmd+V".
    Toast(String),
    /// The newest transcripts, for the pill menu.
    Recent(Vec<String>),
    /// What a trial dictation heard. It was not inserted.
    Trial(String),
    /// A trial ended without text. A cancelled one reports nothing.
    TrialFailed { part: TrialPart, reason: String },
    /// A dictation reached the focused app.
    Inserted(Report),
    /// The user fixed the spelling of one word of a dictation. This word belongs in the
    /// dictionary, and the flow takes no step of its own to put it there.
    WordLearned(String),
    /// The models are gone from memory, so this is a good moment to hand freed pages back.
    Unloaded,
    /// A meeting recording started or stopped.
    Meeting(bool),
    /// A meeting app started using the mic. Offer to take notes, naming this app. Only the
    /// user's answer, through [`Dictation::toggle_meeting`], starts anything.
    SuggestMeeting(String),
    /// Something the Meetings window shows.
    Meetings(MeetingNews),
}

/// The part of the app a failed trial points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrialPart {
    Microphone,
    Models,
}

/// How one dictation went.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// Release of the shortcut to the start of insertion.
    pub release_to_insert: Duration,
    pub timings: wf_history::Timings,
    /// Why LLM cleanup's reply wasn't used, if it ran and wasn't.
    pub cleanup_fallback: Option<String>,
}

/// Everything the flow touches.
pub struct Parts {
    pub settings: Settings,
    pub catalog: Arc<Catalog>,
    /// Where models live unless the settings name a folder.
    pub default_models: PathBuf,
    pub history: PathBuf,
    pub meetings: PathBuf,
    pub microphone: Arc<dyn Microphone>,
    pub meeting_room: Arc<dyn MeetingRoom>,
    pub engines: Arc<dyn Engines>,
    pub context: Arc<dyn ContextSource + Send + Sync>,
    pub media: Arc<dyn MediaControl>,
    /// Where outputs we muted are written down, so a crash can't leave them muted.
    pub media_note: PathBuf,
    /// Takes a builder because the inserter should be made on the thread it lives on.
    pub insertion: Box<dyn FnOnce() -> Box<dyn Insertion> + Send>,
    pub clock: Arc<dyn Clock>,
    pub on_event: Arc<dyn Fn(Event) + Send + Sync>,
}

impl Parts {
    /// The real machine. `executable` is the app's binary, for engine hosts.
    pub fn system(
        settings: Settings,
        catalog: Arc<Catalog>,
        default_models: PathBuf,
        history: PathBuf,
        meetings: PathBuf,
        executable: PathBuf,
        on_event: Arc<dyn Fn(Event) + Send + Sync>,
    ) -> Self {
        Self {
            settings,
            catalog,
            default_models,
            media_note: history.with_file_name("media-note.json"),
            history,
            meetings,
            microphone: Arc::new(SystemMicrophone),
            meeting_room: Arc::new(SystemMeetingRoom),
            engines: Arc::new(remote::ChildEngines::new(executable)),
            context: Arc::new(SystemContext),
            media: Arc::new(SystemMedia),
            insertion: Box::new(|| Box::new(SystemInserter::system())),
            clock: Arc::new(SystemClock::default()),
            on_event,
        }
    }
}

/// The running flow. Cheap to clone; nothing here blocks, so it is safe on the key thread and the
/// UI thread. The flow stops when the last clone is dropped.
#[derive(Clone)]
pub struct Dictation {
    shared: Arc<Shared>,
}

struct Shared {
    inbox: Sender<Input>,
    machine: Arc<Mutex<DictationMachine>>,
    clock: Arc<dyn Clock>,
    keys_paused: AtomicBool,
    meetings: Meetings,
}

impl Drop for Shared {
    fn drop(&mut self) {
        let _ = self.inbox.send(Input::Shutdown);
    }
}

impl Dictation {
    pub fn start(parts: Parts) -> Self {
        let machine = Arc::new(Mutex::new(DictationMachine::new(
            parts.settings.shortcuts.clone(),
        )));
        let (inbox, queue) = mpsc::channel();
        let clock = Arc::clone(&parts.clock);
        let coordinator = Coordinator::new(parts, Arc::clone(&machine), inbox.clone());
        let meetings = coordinator.meetings();
        thread::Builder::new()
            .name("wf-dictation".into())
            .spawn(move || coordinator.run(queue))
            .expect("the dictation thread starts");
        Self {
            shared: Arc::new(Shared {
                inbox,
                machine,
                clock,
                keys_paused: AtomicBool::new(false),
                meetings,
            }),
        }
    }

    /// Feeds one key signal and returns whether to keep it from the focused app. Fast enough for
    /// the key backend's own thread.
    pub fn on_key(&self, signal: KeySignal) -> bool {
        if self.shared.keys_paused.load(Ordering::Relaxed) {
            return false;
        }
        let now = self.shared.clock.now();
        let mut machine = self.lock_machine();
        let (commands, swallow) = match signal {
            KeySignal::Key(event) => {
                let reaction = machine.on_key(event, now);
                (reaction.commands, reaction.swallow)
            }
            KeySignal::Lost => (machine.on_keys_lost(now), false),
        };
        let deadline = machine.next_deadline();
        drop(machine);
        if !commands.is_empty() || deadline.is_some() {
            self.send(Input::Commands(commands));
        }
        swallow
    }

    /// A click on the pill.
    pub fn on_pill(&self, input: PillInput) {
        let commands = self.lock_machine().on_pill(input, self.shared.clock.now());
        self.send(Input::Commands(commands));
    }

    /// The Settings window is recording a shortcut, so keys are not for us. Nothing is known
    /// about held keys afterwards.
    pub fn pause_keys(&self, paused: bool) {
        self.shared.keys_paused.store(paused, Ordering::Relaxed);
        if !paused {
            let commands = self.lock_machine().on_keys_lost(self.shared.clock.now());
            self.send(Input::Commands(commands));
        }
    }

    /// Applies new settings at once: shortcuts, retention and models. A dictation in progress
    /// finishes with the settings it started on.
    pub fn settings_changed(&self, settings: Settings) {
        self.send(Input::Settings(Box::new(settings)));
    }

    /// The app can't work until this clears, such as when it can't see the keyboard. The pill
    /// shows the reason for as long as it lasts.
    pub fn set_blocked(&self, reason: Option<String>) {
        self.send(Input::Blocked(reason));
    }

    pub fn paste_last(&self) {
        self.send(Input::PasteLast);
    }

    /// Pastes an entry of the last [`Event::Recent`] list.
    pub fn paste_recent(&self, index: usize) {
        self.send(Input::PasteRecent(index));
    }

    /// Forgets all transcripts and closes the history file, for "Delete all data", which removes
    /// its folder next. Waits for the insertion in progress, if any, for at most a second or two.
    pub fn forget_history(&self) {
        let (done, wait) = mpsc::channel();
        self.send(Input::ForgetHistory(done));
        let _ = wait.recv_timeout(FORGET_LIMIT);
    }

    /// Starts or stops the meeting notes.
    pub fn toggle_meeting(&self) {
        self.send(Input::ToggleMeeting);
    }

    /// Records more of a meeting that already ended.
    pub fn resume_meeting(&self, id: &str) {
        self.shared.meetings.resume(id);
    }

    /// Runs the stages a meeting never finished, such as after a crash.
    pub fn finish_meeting(&self, id: &str) {
        self.shared.meetings.finish_stages(id);
    }

    /// Writes the summary again, from scratch or reusing what hasn't changed.
    pub fn summarize_meeting(&self, id: &str, mode: SummaryMode) {
        self.shared.meetings.write_summary(id, mode);
    }

    /// Stops the work on a meeting. What's done stays.
    pub fn cancel_meeting_work(&self, id: &str) {
        self.shared.meetings.cancel_work(id);
    }

    /// Asks a question about a meeting. The answer arrives as [`MeetingNews::Chat`].
    pub fn ask_meeting(&self, id: &str, question: &str) -> ChatRun {
        self.shared.meetings.ask(id, question)
    }

    /// Which meeting is recording and which are being worked on.
    pub fn meetings_status(&self) -> MeetingsStatus {
        self.shared.meetings.status()
    }

    /// Finishes a running recording, for when the app is going away. Waits for at most `limit`.
    /// What doesn't make it in time is recovered from the file at the next launch.
    pub fn close_meetings(&self, limit: Duration) {
        let (done, wait) = mpsc::channel();
        self.send(Input::CloseMeetings(done));
        let _ = wait.recv_timeout(limit);
    }

    /// Puts back whatever a dictation in progress muted or paused, and leaves media alone from
    /// here on, for when the app is going away. Waits for at most `limit`. A mute that doesn't
    /// come off in time comes off at the next launch.
    pub fn close_media(&self, limit: Duration) {
        let (done, wait) = mpsc::channel();
        self.send(Input::CloseMedia(done));
        let _ = wait.recv_timeout(limit);
    }

    /// Starts a real dictation that ends in [`Event::Trial`] or [`Event::TrialFailed`] rather than
    /// in the focused app. Finish it with [`PillInput::Stop`] or [`PillInput::Cancel`], through
    /// [`Self::on_pill`]. Onboarding's "try a dictation" step falls back on this when the shortcut
    /// doesn't work, which it can't prove by itself.
    pub fn start_trial(&self) {
        self.send(Input::Trial);
    }

    /// While armed, every dictation the shortcut starts ends like one from [`Self::start_trial`].
    /// Disarm it, or the user's dictations go nowhere.
    pub fn arm_trial(&self, armed: bool) {
        self.send(Input::ArmTrial(armed));
    }

    /// Wakes the flow to look at its clock. Tests call this after moving a fake one.
    pub fn poke(&self) {
        self.send(Input::Commands(Vec::new()));
    }

    fn lock_machine(&self) -> std::sync::MutexGuard<'_, DictationMachine> {
        self.shared
            .machine
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn send(&self, input: Input) {
        let _ = self.shared.inbox.send(input);
    }
}
