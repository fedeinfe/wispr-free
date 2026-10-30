//! The flow's one decision-maker: a thread that owns the state of the dictation and turns inputs
//! into work for the other threads. Nothing here waits on a mic, an engine or the clipboard.

use std::mem;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use wf_core::dictation::{CancelReason, Command, DictationMachine, Mode, PillInput};
use wf_core::pill::PillState;
use wf_core::settings::{HistoryRetention, KeepModels, Settings};
use wf_models::{Catalog, ModelStore};

use crate::engines::{LlmSlot, Loaded};
use crate::meeting::{Meetings, Office};
use crate::plan::plan;
use crate::session::{Pump, Session, SessionShared};
use crate::worker::{self, Job};
use crate::{Clock, Event, Microphone, Parts};

/// How long the pill keeps showing an error, unless a new dictation starts first.
const ERROR_SHOWN: Duration = Duration::from_secs(6);
/// The "5 min" of the keep models loaded setting.
const KEEP_LOADED_FOR: Duration = Duration::from_secs(5 * 60);
/// How often expired history is removed while the app sits idle.
const PRUNE_EVERY: Duration = Duration::from_secs(60 * 60);

const STRAY_KEY_HINT: &str = "Dictation discarded: another key was pressed";
const CAP_HINT: &str = "One minute of dictation left";

pub(crate) enum Input {
    Commands(Vec<Command>),
    Settings(Box<Settings>),
    Blocked(Option<String>),
    PasteLast,
    PasteRecent(usize),
    ForgetHistory(Sender<()>),
    Trial,
    ToggleMeeting,
    /// Closes any recording for good, and says when that's done.
    CloseMeetings(Sender<()>),
    Meeting(bool),
    /// Meeting work let go of the model.
    LlmReturned,
    Problem(String),
    AutoStop(u64),
    /// A dictation is over: inserted, abandoned or failed. `problem` is for the pill.
    Finished {
        id: u64,
        problem: Option<String>,
    },
    Shutdown,
}

struct Recording {
    id: u64,
    mode: Mode,
    pump: Arc<Pump>,
    cancel: Arc<AtomicBool>,
    locked: Arc<AtomicBool>,
}

/// A recording that ended and is on its way to insertion.
struct InFlight {
    id: u64,
    cancel: Arc<AtomicBool>,
}

pub(crate) struct Coordinator {
    machine: Arc<Mutex<DictationMachine>>,
    clock: Arc<dyn Clock>,
    catalog: Arc<Catalog>,
    default_models: PathBuf,
    settings: Settings,
    store: ModelStore,
    microphone: Arc<dyn Microphone>,
    shared: Arc<SessionShared>,
    next_id: u64,
    recording: Option<Recording>,
    in_flight: Vec<InFlight>,
    trial_next: bool,
    error: Option<(String, Duration)>,
    blocked: Option<String>,
    meetings: Meetings,
    meeting: bool,
    shown: PillState,
    unload_at: Option<Duration>,
    prune_at: Option<Duration>,
}

impl Coordinator {
    pub(crate) fn new(
        parts: Parts,
        machine: Arc<Mutex<DictationMachine>>,
        inbox: Sender<Input>,
    ) -> Self {
        let (jobs, queue) = mpsc::channel();
        let store = ModelStore::configured(
            parts.settings.pipeline.models_folder.as_deref(),
            &parts.default_models,
        );
        let on_event = Arc::clone(&parts.on_event);
        let finished = inbox.clone();
        let llm = Arc::new(LlmSlot::new(&parts.engines, inbox.clone()));
        let meetings = Meetings::spawn(
            parts.meeting_room,
            Arc::new(Office::new(
                parts.meetings,
                parts.settings.clone(),
                Arc::clone(&parts.catalog),
                parts.default_models.clone(),
                Arc::clone(&parts.engines),
                Arc::clone(&llm),
                Arc::clone(&parts.clock),
                inbox.clone(),
                Arc::clone(&parts.on_event),
            )),
        );
        let (insertion, history) = (parts.insertion, parts.history);
        thread::Builder::new()
            .name("wf-insertion".into())
            .spawn(move || worker::run(queue, insertion(), history, finished, on_event))
            .expect("the insertion thread starts");

        let retention = parts.settings.privacy.history_retention;
        let _ = jobs.send(Job::SetRetention(retention));
        let shared = Arc::new(SessionShared {
            engines: parts.engines,
            loaded: Mutex::default(),
            llm,
            context: parts.context,
            jobs,
            inbox,
            on_event: parts.on_event,
        });
        let mut coordinator = Self {
            machine,
            clock: parts.clock,
            catalog: parts.catalog,
            default_models: parts.default_models,
            settings: parts.settings,
            store,
            microphone: parts.microphone,
            shared,
            next_id: 0,
            recording: None,
            in_flight: Vec::new(),
            trial_next: false,
            error: None,
            blocked: None,
            meetings,
            meeting: false,
            shown: PillState::Resting,
            unload_at: None,
            prune_at: None,
        };
        coordinator.schedule_prune();
        coordinator
    }

    /// The way in to the meetings, for the handle the app holds.
    pub(crate) fn meetings(&self) -> Meetings {
        self.meetings.clone()
    }

    pub(crate) fn run(mut self, queue: Receiver<Input>) {
        // The first check can be slow, and the next one comes on a key-down.
        let _ = self.microphone.problem();
        loop {
            let input = match self.next_wake() {
                Some(wait) => queue.recv_timeout(wait),
                None => queue.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match input {
                Ok(Input::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(input) => self.handle(input),
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.on_time();
            self.show();
        }
        self.stop_recording(Pump::DISCARD);
    }

    fn lock_machine(&self) -> MutexGuard<'_, DictationMachine> {
        self.machine.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn next_wake(&self) -> Option<Duration> {
        let now = self.clock.now();
        let error = self.error.as_ref().map(|(_, until)| *until);
        let machine = self.lock_machine().next_deadline();
        [machine, error, self.unload_at, self.prune_at]
            .into_iter()
            .flatten()
            .min()
            .map(|deadline| deadline.saturating_sub(now))
    }

    fn on_time(&mut self) {
        let now = self.clock.now();
        let commands = self.lock_machine().tick(now);
        self.on_commands(commands);
        if self.error.as_ref().is_some_and(|(_, until)| now >= *until) {
            self.error = None;
        }
        if self.unload_at.is_some_and(|at| now >= at) {
            self.unload();
        }
        if self.prune_at.is_some_and(|at| now >= at) {
            self.send_job(Job::Prune);
            self.schedule_prune();
        }
    }

    fn handle(&mut self, input: Input) {
        match input {
            Input::Commands(commands) => self.on_commands(commands),
            Input::Settings(settings) => self.on_settings(*settings),
            Input::Blocked(reason) => self.blocked = reason,
            Input::PasteLast => self.send_job(Job::PasteLast(self.paste_hint())),
            Input::PasteRecent(index) => self.send_job(Job::PasteRecent(index)),
            Input::ForgetHistory(done) => self.send_job(Job::Forget(done)),
            Input::ToggleMeeting => self.meetings.toggle(),
            Input::CloseMeetings(done) => self.meetings.close(done),
            Input::LlmReturned => self.after_use(),
            Input::Meeting(active) => {
                self.meeting = active;
                (self.shared.on_event)(Event::Meeting(active));
            }
            Input::Problem(problem) => self.fail(problem),
            Input::Trial => {
                let now = self.clock.now();
                let commands = self.lock_machine().on_pill(PillInput::Click, now);
                self.trial_next = commands.iter().any(|c| matches!(c, Command::Start(_)));
                self.on_commands(commands);
            }
            Input::AutoStop(id) => {
                if self.recording.as_ref().is_some_and(|r| r.id == id) {
                    let now = self.clock.now();
                    let commands = self.lock_machine().on_pill(PillInput::Stop, now);
                    self.on_commands(commands);
                }
            }
            Input::Finished { id, problem } => self.on_finished(id, problem),
            Input::Shutdown => {}
        }
    }

    fn on_commands(&mut self, commands: Vec<Command>) {
        for command in commands {
            match command {
                Command::Start(mode) => self.start(mode),
                Command::Lock => {
                    if let Some(recording) = &mut self.recording {
                        recording.mode = Mode::HandsFree;
                        recording.locked.store(true, Ordering::Relaxed);
                    }
                }
                Command::StopAndInsert => self.stop(),
                Command::Cancel(reason) => self.cancel(reason),
                Command::CapWarning => self.toast(CAP_HINT),
                Command::PasteLast => self.send_job(Job::PasteLast(self.paste_hint())),
                Command::CopyLast => self.send_job(Job::CopyLast),
                Command::ToggleMeetingNotes => self.meetings.toggle(),
            }
        }
    }

    fn start(&mut self, mode: Mode) {
        self.error = None;
        self.unload_at = None;
        let trial = mem::take(&mut self.trial_next);
        let plan = match plan(&self.settings, &self.catalog, &self.store) {
            Ok(plan) => plan,
            Err(problem) => return self.refuse(problem),
        };
        if let Some(problem) = self.microphone.problem() {
            return self.refuse(problem);
        }
        self.next_id += 1;
        let id = self.next_id;
        let (audio, heard) = mpsc::channel();
        let pump = Arc::new(Pump::default());
        let cancel = Arc::<AtomicBool>::default();
        let locked = Arc::new(AtomicBool::new(mode == Mode::HandsFree));
        pump.spawn(
            Arc::clone(&self.microphone),
            self.settings.general.microphones.clone(),
            audio,
            id,
            Arc::clone(&self.shared),
        );
        Session {
            id,
            settings: self.settings.clone(),
            plan,
            heard,
            cancel: Arc::clone(&cancel),
            locked: Arc::clone(&locked),
            trial,
            shared: Arc::clone(&self.shared),
        }
        .spawn();
        self.recording = Some(Recording {
            id,
            mode,
            pump,
            cancel,
            locked,
        });
    }

    /// Ends a dictation that can't go on, with the keys possibly still down.
    fn refuse(&mut self, problem: String) {
        self.lock_machine().abandon();
        self.fail(problem);
    }

    fn fail(&mut self, problem: String) {
        self.error = Some((problem, self.clock.now() + ERROR_SHOWN));
    }

    fn stop(&mut self) {
        if let Some(recording) = self.recording.take() {
            recording.pump.finish(Instant::now());
            self.in_flight.push(InFlight {
                id: recording.id,
                cancel: recording.cancel,
            });
        }
    }

    fn cancel(&mut self, reason: CancelReason) {
        if reason == CancelReason::StrayKey {
            self.toast(STRAY_KEY_HINT);
        }
        if self.stop_recording(Pump::DISCARD) {
            return;
        }
        // Past the recording, so the work is in an engine. Stopping it costs a reload.
        for dictation in &self.in_flight {
            dictation.cancel.store(true, Ordering::SeqCst);
        }
        if !self.in_flight.is_empty() {
            self.shared.engines.interrupt();
            // With meeting work on it, the model may be busy with someone else's call. A cleanup
            // waiting behind that one ends by its own timeout.
            if !self.shared.llm.is_leased() {
                self.shared.llm.interrupt();
            }
        }
    }

    /// Returns whether there was a recording.
    fn stop_recording(&mut self, how: u8) -> bool {
        let Some(recording) = self.recording.take() else {
            return false;
        };
        recording.cancel.store(true, Ordering::SeqCst);
        recording.pump.stop(how);
        self.in_flight.push(InFlight {
            id: recording.id,
            cancel: recording.cancel,
        });
        true
    }

    fn on_finished(&mut self, id: u64, problem: Option<String>) {
        if self.recording.as_ref().is_some_and(|r| r.id == id) {
            self.lock_machine().abandon();
            self.stop_recording(Pump::DISCARD);
        }
        self.in_flight.retain(|dictation| dictation.id != id);
        if let Some(problem) = problem {
            self.fail(problem);
        }
        self.after_use();
    }

    /// Applies "keep models loaded" once nothing is using them.
    fn after_use(&mut self) {
        if self.is_idle() {
            match self.settings.pipeline.keep_models_loaded {
                KeepModels::Always => {}
                KeepModels::FiveMinutes => {
                    self.unload_at = Some(self.clock.now() + KEEP_LOADED_FOR)
                }
                KeepModels::UnloadAfterUse => self.unload(),
            }
        }
    }

    fn is_idle(&self) -> bool {
        self.recording.is_none() && self.in_flight.is_empty() && !self.shared.llm.is_leased()
    }

    fn unload(&mut self) {
        self.unload_at = None;
        if !self.is_idle() {
            return;
        }
        let Ok(mut loaded) = self.shared.loaded.try_lock() else {
            return;
        };
        if loaded.is_empty() && !self.shared.llm.is_loaded() {
            return;
        }
        let dropped: Loaded = mem::take(&mut *loaded);
        drop(loaded);
        let llm = self.shared.llm.clear();
        // Ending a process with a model mapped can take a moment.
        let on_event = Arc::clone(&self.shared.on_event);
        thread::spawn(move || {
            drop(dropped);
            drop(llm);
            on_event(Event::Unloaded);
        });
    }

    fn on_settings(&mut self, settings: Settings) {
        let before = mem::replace(&mut self.settings, settings);
        let settings = &self.settings;
        self.meetings.settings_changed(settings);
        self.lock_machine().set_bindings(settings.shortcuts.clone());
        if settings.pipeline.models_folder != before.pipeline.models_folder {
            self.store = ModelStore::configured(
                settings.pipeline.models_folder.as_deref(),
                &self.default_models,
            );
        }
        let retention = settings.privacy.history_retention;
        if retention != before.privacy.history_retention {
            self.send_job(Job::SetRetention(retention));
            self.schedule_prune();
        }
        let keep = self.settings.pipeline.keep_models_loaded;
        if keep != before.pipeline.keep_models_loaded && self.is_idle() {
            match keep {
                KeepModels::Always => self.unload_at = None,
                KeepModels::FiveMinutes => {
                    self.unload_at = Some(self.clock.now() + KEEP_LOADED_FOR)
                }
                KeepModels::UnloadAfterUse => self.unload(),
            }
        }
    }

    fn schedule_prune(&mut self) {
        let retention = self.settings.privacy.history_retention;
        self.prune_at =
            (retention == HistoryRetention::Delete24h).then(|| self.clock.now() + PRUNE_EVERY);
    }

    fn send_job(&self, job: Job) {
        let _ = self.shared.jobs.send(job);
    }

    fn paste_hint(&self) -> String {
        worker::paste_hint(&self.settings)
    }

    fn toast(&self, text: &str) {
        (self.shared.on_event)(Event::Toast(text.to_owned()));
    }

    /// Reports the pill's state if it changed, and tells the machine whether Cancel still applies.
    fn show(&mut self) {
        let processing = self.visible_in_flight();
        self.lock_machine().set_processing(processing);
        let state = match (&self.recording, &self.error, &self.blocked) {
            (Some(recording), ..) => match recording.mode {
                Mode::PushToTalk => PillState::ListeningPtt,
                Mode::HandsFree => PillState::ListeningHandsFree,
            },
            _ if processing => PillState::Processing,
            (_, Some((reason, _)), _) | (_, None, Some(reason)) => PillState::Error {
                reason: reason.clone(),
            },
            _ if self.meeting => PillState::Meeting,
            _ => PillState::Resting,
        };
        if state != self.shown {
            self.shown = state.clone();
            (self.shared.on_event)(Event::Pill(state));
        }
    }

    fn visible_in_flight(&self) -> bool {
        self.in_flight
            .iter()
            .any(|dictation| !dictation.cancel.load(Ordering::SeqCst))
    }
}
