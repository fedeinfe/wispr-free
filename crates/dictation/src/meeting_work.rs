//! What takes a meeting from stopped to ready, and what the window asks for afterwards. Work on
//! meetings runs one job at a time on a thread of its own, so a long diarization holds up nothing
//! but the next meeting's. A chat answer doesn't queue here: it only needs the shared LLM, and takes
//! its turn there, so it works while a summary runs.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::thread;
use std::time::Duration;

use wf_core::settings::Settings;
use wf_meetings::{
    ChatOptions, ChatSource, ChatTurn, Error, LiveTranscript, Meeting, MeetingEngines,
    ProcessOptions, ProcessProgress, RefineOptions, Stage, StageState, SummaryMode, SummaryOptions,
    SummaryProgress, ask, finish_meeting, generate_summary,
};
use wf_pipeline::{Asr, AsrOptions, Diarizer, Llm, PipelineError, Vad};

use crate::engines::Engines;
use crate::meeting::{ChatUpdate, MeetingNews, Office};
use crate::plan::{
    AsrModel, DiarizerModel, LlmModel, meeting_diarizer, meeting_llm, meeting_plan, meeting_speech,
    single_language,
};

const READY: &str = "Meeting notes are ready";
const READY_WITH_PROBLEM: &str = "Meeting notes are ready, but a step failed";

/// Where a meeting's work has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkStep {
    Waiting,
    /// The last of the live transcript is being written.
    Finishing,
    Stage(Stage),
    Refining {
        done_ms: u64,
        total_ms: u64,
    },
    Summarizing {
        done: usize,
        total: usize,
    },
}

enum Job {
    /// A recording just stopped. `length` is `None` if it stopped badly.
    Close {
        meeting: Box<Meeting>,
        live: Option<LiveTranscript>,
        length: Option<Duration>,
    },
    /// Runs the stages that haven't run.
    Finish,
    Summary(SummaryMode),
}

/// A job's place in line, and how to stop it.
#[derive(Clone)]
pub(crate) struct Ticket {
    id: String,
    cancel: Arc<AtomicBool>,
    /// Interrupting what's in this scope touches nothing else.
    engines: Arc<dyn Engines>,
}

/// One job for the work thread.
pub(crate) struct Work {
    ticket: Ticket,
    job: Job,
}

/// A chat answer on its way. Stopping it keeps what came so far.
pub struct ChatRun {
    /// What its [`ChatUpdate`]s are marked with.
    pub run: u64,
    cancel: Arc<AtomicBool>,
}

impl ChatRun {
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

pub(crate) fn run(office: &Arc<Office>, jobs: Receiver<Work>) {
    for work in jobs {
        let id = work.ticket.id.clone();
        let problem = office.perform(work);
        office.leave_line(&id);
        office.news(MeetingNews::WorkDone { id, problem });
    }
}

impl Office {
    /// Hands a job to the work thread and marks the meeting busy straight away.
    pub(crate) fn close_meeting(
        &self,
        jobs: &Sender<Work>,
        meeting: Meeting,
        live: Option<LiveTranscript>,
        length: Option<Duration>,
    ) {
        let id = meeting.record().id.clone();
        let job = Job::Close {
            meeting: Box::new(meeting),
            live,
            length,
        };
        self.enqueue(jobs, &id, job);
    }

    pub(crate) fn finish_stages(&self, jobs: &Sender<Work>, id: &str) {
        self.enqueue(jobs, id, Job::Finish);
    }

    pub(crate) fn write_summary(&self, jobs: &Sender<Work>, id: &str, mode: SummaryMode) {
        self.enqueue(jobs, id, Job::Summary(mode));
    }

    fn enqueue(&self, jobs: &Sender<Work>, id: &str, job: Job) {
        if self.is_in_line(id) && !matches!(job, Job::Close { .. }) {
            return;
        }
        let ticket = Ticket {
            id: id.to_owned(),
            cancel: Arc::default(),
            engines: self.engines.scope(),
        };
        self.tickets().push(ticket.clone());
        self.status().work.push((id.to_owned(), WorkStep::Waiting));
        self.news(MeetingNews::Working {
            id: id.to_owned(),
            step: WorkStep::Waiting,
        });
        let _ = jobs.send(Work { ticket, job });
    }

    fn is_in_line(&self, id: &str) -> bool {
        self.tickets().iter().any(|ticket| ticket.id == id)
    }

    fn leave_line(&self, id: &str) {
        self.tickets().retain(|ticket| ticket.id != id);
        self.status().work.retain(|(busy, _)| busy != id);
    }

    /// Stops what's running on a meeting, or takes it out of the line. What's done stays, and the
    /// stages not reached stay pending.
    pub(crate) fn cancel_work(&self, id: &str) {
        for ticket in self.tickets().iter().filter(|ticket| ticket.id == id) {
            ticket.cancel.store(true, Ordering::SeqCst);
            ticket.engines.interrupt();
        }
    }

    fn step(&self, id: &str, step: WorkStep) {
        let mut status = self.status();
        if let Some(entry) = status.work.iter_mut().find(|(busy, _)| busy == id) {
            entry.1 = step;
        }
        drop(status);
        self.news(MeetingNews::Working {
            id: id.to_owned(),
            step,
        });
    }

    /// Returns what went wrong, for the window. Each stage's own failure is in the record.
    fn perform(self: &Arc<Self>, work: Work) -> Option<String> {
        let Work { ticket, job } = work;
        let result = match job {
            Job::Close {
                meeting,
                live,
                length,
            } => self.close(*meeting, live, length, &ticket),
            Job::Finish => self
                .store
                .open(&ticket.id)
                .and_then(|mut meeting| self.finish(&mut meeting, &ticket)),
            Job::Summary(mode) => self
                .store
                .open(&ticket.id)
                .and_then(|mut meeting| self.summarize(&mut meeting, mode, &ticket)),
        };
        match result {
            Ok(()) | Err(Error::Cancelled) => None,
            Err(error) => Some(error.to_string()),
        }
    }

    fn close(
        self: &Arc<Self>,
        mut meeting: Meeting,
        live: Option<LiveTranscript>,
        length: Option<Duration>,
        ticket: &Ticket,
    ) -> Result<(), Error> {
        self.step(&ticket.id, WorkStep::Finishing);
        if let Some(live) = live {
            // This can only cost the end of the live text, and the audio still has it.
            if let Err(error) = live.finish() {
                eprintln!("the live transcript ended badly: {error}");
            }
        }
        match length {
            Some(length) => meeting.end_part(length)?,
            None => meeting.recover_part()?,
        }
        self.finish(&mut meeting, ticket)
    }

    fn finish(self: &Arc<Self>, meeting: &mut Meeting, ticket: &Ticket) -> Result<(), Error> {
        let settings = self.settings();
        let models = self.models(&settings);
        let mut plan = meeting_plan(&settings, &self.catalog);
        // With no live text, the refine pass is the only way to a transcript.
        plan.refine |= meeting.read_live()?.segments.is_empty();
        let mut engines = StageEngines {
            office: Arc::clone(self),
            engines: Arc::clone(&ticket.engines),
            cancel: Arc::clone(&ticket.cancel),
            speech: meeting_speech(&settings, &self.catalog, &models),
            diarizer: meeting_diarizer(&settings, &self.catalog, &models),
            llm: meeting_llm(&settings, &self.catalog, &models),
        };
        let id = ticket.id.clone();
        finish_meeting(
            meeting,
            plan,
            &mut engines,
            &process_options(&settings),
            &ticket.cancel,
            &mut |progress| self.step(&id, step_of(progress)),
        )?;
        let stages = &meeting.record().stages;
        let failed = [&stages.diarization, &stages.refine, &stages.summary]
            .iter()
            .any(|state| matches!(state, StageState::Failed { .. }));
        self.toast(if failed { READY_WITH_PROBLEM } else { READY });
        Ok(())
    }

    fn summarize(
        self: &Arc<Self>,
        meeting: &mut Meeting,
        mode: SummaryMode,
        ticket: &Ticket,
    ) -> Result<(), Error> {
        let settings = self.settings();
        let models = self.models(&settings);
        let model = match meeting_llm(&settings, &self.catalog, &models) {
            Ok(model) => model,
            Err(problem) => {
                let reason = PipelineError::ModelMissing(problem);
                let state = StageState::Failed {
                    reason: reason.to_string(),
                };
                meeting.set_stage(Stage::Summary, state)?;
                return Err(reason.into());
            }
        };
        let mut llm = self.llm.lease(&model, Arc::clone(&ticket.cancel));
        let id = ticket.id.clone();
        generate_summary(
            meeting,
            &mut llm,
            mode,
            &process_options(&settings).summary,
            &ticket.cancel,
            &mut |SummaryProgress { done, total }| {
                self.step(&id, WorkStep::Summarizing { done, total });
            },
        )?;
        Ok(())
    }

    /// Answers on a thread of its own. Tokens and the end arrive as [`MeetingNews::Chat`].
    pub(crate) fn ask(self: &Arc<Self>, id: &str, question: &str) -> ChatRun {
        let run = self.next_chat_run();
        let cancel = Arc::<AtomicBool>::default();
        let office = Arc::clone(self);
        let (id, question, stop) = (id.to_owned(), question.to_owned(), Arc::clone(&cancel));
        thread::Builder::new()
            .name("wf-meeting-chat".into())
            .spawn(move || {
                let answer = office.answer(&id, &question, run, stop);
                let update = ChatUpdate::Done(answer);
                office.news(MeetingNews::Chat { run, update });
            })
            .expect("the chat thread starts");
        ChatRun { run, cancel }
    }

    fn answer(
        &self,
        id: &str,
        question: &str,
        run: u64,
        cancel: Arc<AtomicBool>,
    ) -> Result<ChatTurn, String> {
        let settings = self.settings();
        let models = self.models(&settings);
        let model = meeting_llm(&settings, &self.catalog, &models)
            .map_err(|problem| format!("The chat needs a model. {problem}"))?;
        let describe = |error: Error| error.to_string();
        let mut meeting = self.store.open(id).map_err(describe)?;
        let source = ChatSource::of(&meeting).map_err(describe)?;
        let mut llm = self.llm.lease(&model, cancel);
        let turn = ask(
            &mut llm,
            &source,
            question,
            &ChatOptions::default(),
            &mut |token| {
                let update = ChatUpdate::Token(token.to_owned());
                self.news(MeetingNews::Chat { run, update });
            },
        )
        .map_err(describe)?;
        drop(llm);
        if turn.answer.is_empty() && !turn.stopped {
            return Err("The model gave no answer".to_owned());
        }
        meeting.add_chat_turn(turn.clone()).map_err(describe)?;
        Ok(turn)
    }
}

fn step_of(progress: ProcessProgress) -> WorkStep {
    match progress {
        ProcessProgress::Stage(stage) => WorkStep::Stage(stage),
        ProcessProgress::Refining { done_ms, total_ms } => WorkStep::Refining { done_ms, total_ms },
        ProcessProgress::Summarizing(SummaryProgress { done, total }) => {
            WorkStep::Summarizing { done, total }
        }
    }
}

/// The Meetings settings and "My languages", for the stages.
fn process_options(settings: &Settings) -> ProcessOptions {
    let language = single_language(settings);
    ProcessOptions {
        refine: RefineOptions {
            asr: AsrOptions {
                language: language.clone(),
                bias_words: settings.style.dictionary.clone(),
            },
            ..RefineOptions::default()
        },
        summary: SummaryOptions {
            language,
            instructions: settings.meetings.summary_template.clone(),
            ..SummaryOptions::default()
        },
    }
}

/// The stages' models, loaded as each stage starts. A model that can't be used is that stage's
/// failure and no other's.
struct StageEngines {
    office: Arc<Office>,
    engines: Arc<dyn Engines>,
    cancel: Arc<AtomicBool>,
    speech: Result<(AsrModel, PathBuf), String>,
    diarizer: Result<DiarizerModel, String>,
    llm: Result<LlmModel, String>,
}

fn usable<T: Clone>(model: &Result<T, String>) -> Result<T, PipelineError> {
    model.clone().map_err(PipelineError::ModelMissing)
}

impl MeetingEngines for StageEngines {
    fn asr(&mut self) -> Result<Box<dyn Asr>, PipelineError> {
        self.engines.asr(&usable(&self.speech)?.0)
    }

    fn vad(&mut self) -> Result<Box<dyn Vad>, PipelineError> {
        self.engines.vad(&usable(&self.speech)?.1)
    }

    fn diarizer(&mut self) -> Result<Box<dyn Diarizer>, PipelineError> {
        self.engines.diarizer(&usable(&self.diarizer)?)
    }

    fn llm(&mut self) -> Result<Box<dyn Llm>, PipelineError> {
        let model = usable(&self.llm)?;
        let cancel = Arc::clone(&self.cancel);
        Ok(Box::new(self.office.llm.lease(&model, cancel)))
    }
}
