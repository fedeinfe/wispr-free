//! One dictation from key-down on: the mic pump, and the session that turns its audio into the
//! text to insert. Audio flows the whole time, so ASR can start before the user stops. Frames wait
//! in a channel while the engines load, so a first dictation loses nothing to the load.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, RecvError, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use wf_audio::{CaptureEvent, Denoiser, LevelMeter};
use wf_core::audio::{AudioClip, SAMPLE_RATE_HZ};
use wf_core::settings::{Settings, Style, StyleSettings};
use wf_history::Timings;
use wf_pipeline::cleanup::{CleanupRequest, DEFAULT_TIMEOUT, Fallback, llm_cleanup};
use wf_pipeline::passes::{AppCategory, LocalPassOptions, app_category, local_passes};
use wf_pipeline::rules::rule_cleanup;
use wf_pipeline::vad::{LiveVad, VadOptions, speech_chunks, transcribe_speech};
use wf_pipeline::worker::AsrWorker;
use wf_pipeline::{Asr, AsrOptions, PipelineError, Transcript, Vad};
use wf_platform::context::{Context, ContextDepth, ContextSource, Focus, TextAround};

use crate::coordinator::Input;
use crate::engines::{Engines, Loaded, Ready};
use crate::plan::Plan;
use crate::worker::{Insert, Job};
use crate::{Event, Microphone};

/// How often the pump looks at its stop flag while the mic is quiet.
const STOP_POLL: Duration = Duration::from_millis(10);
/// A mic that opened but sent nothing by now isn't going to.
const FIRST_FRAME_LIMIT: Duration = Duration::from_secs(3);
/// On-screen words offered to ASR, nearest the cursor first in what's kept.
const MAX_CONTEXT_WORDS: usize = 40;

const NO_MIC: &str = "No microphone found";
const SILENT_MIC: &str = "The microphone sends only silence. Check its permission and mute switch";

/// What the threads of a dictation share with the coordinator.
pub(crate) struct SessionShared {
    pub(crate) engines: Arc<dyn Engines>,
    pub(crate) loaded: Mutex<Loaded>,
    pub(crate) context: Arc<dyn ContextSource + Send + Sync>,
    pub(crate) jobs: Sender<Job>,
    pub(crate) inbox: Sender<Input>,
    pub(crate) on_event: Arc<dyn Fn(Event) + Send + Sync>,
}

impl SessionShared {
    fn finished(&self, id: u64, problem: Option<String>) {
        let _ = self.inbox.send(Input::Finished { id, problem });
    }
}

pub(crate) enum Audio {
    Frame(Vec<f32>),
    End { released: Instant },
}

/// Moves audio from the mic to the session, and owns the open mic.
#[derive(Default)]
pub(crate) struct Pump {
    state: AtomicU8,
    released: OnceLock<Instant>,
}

impl Pump {
    const RUN: u8 = 0;
    const FINISH: u8 = 1;
    pub(crate) const DISCARD: u8 = 2;

    /// Opens the mic on its own thread, so the caller never waits for a device.
    pub(crate) fn spawn(
        self: &Arc<Self>,
        microphone: Arc<dyn Microphone>,
        ranking: Vec<String>,
        audio: Sender<Audio>,
        id: u64,
        shared: Arc<SessionShared>,
    ) {
        let pump = Arc::clone(self);
        thread::Builder::new()
            .name("wf-mic-pump".into())
            .spawn(move || {
                if let Err(problem) = pump.run(microphone.as_ref(), ranking, audio, &shared) {
                    shared.finished(id, Some(problem));
                }
            })
            .expect("the mic thread starts");
    }

    /// Ends the recording, keeping what is already captured.
    pub(crate) fn finish(&self, released: Instant) {
        let _ = self.released.set(released);
        self.stop(Self::FINISH);
    }

    pub(crate) fn stop(&self, how: u8) {
        self.state.store(how, Ordering::SeqCst);
    }

    fn run(
        &self,
        microphone: &dyn Microphone,
        ranking: Vec<String>,
        audio: Sender<Audio>,
        shared: &SessionShared,
    ) -> Result<(), String> {
        let opened = Instant::now();
        let stream = microphone.open(ranking)?;
        let events = stream.events();
        let mut meter = LevelMeter::new();
        let mut started = false;
        let mut forward = |frame: Vec<f32>| {
            if let Some(level) = meter.push(&frame) {
                (shared.on_event)(Event::Level(level));
            }
            let _ = audio.send(Audio::Frame(frame));
        };
        loop {
            match self.state.load(Ordering::SeqCst) {
                Self::RUN => {}
                Self::FINISH => {
                    for event in events.try_iter() {
                        if let CaptureEvent::Frame(frame) = event {
                            forward(frame);
                        }
                    }
                    let released = self.released.get().copied().unwrap_or_else(Instant::now);
                    let _ = audio.send(Audio::End { released });
                    return Ok(());
                }
                _ => return Ok(()),
            }
            match events.recv_timeout(STOP_POLL) {
                Ok(CaptureEvent::Frame(frame)) => {
                    started = true;
                    forward(frame);
                }
                Ok(CaptureEvent::Device(None)) => return Err(NO_MIC.to_owned()),
                Ok(CaptureEvent::Device(Some(_))) => {}
                Err(RecvTimeoutError::Timeout) => {
                    if !started && opened.elapsed() >= FIRST_FRAME_LIMIT {
                        return Err(NO_MIC.to_owned());
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return Err(NO_MIC.to_owned()),
            }
        }
    }
}

pub(crate) struct Session {
    pub(crate) id: u64,
    pub(crate) settings: Settings,
    pub(crate) plan: Plan,
    pub(crate) heard: Receiver<Audio>,
    pub(crate) cancel: Arc<AtomicBool>,
    /// Hands-free right now, so the silence auto-stop applies.
    pub(crate) locked: Arc<AtomicBool>,
    pub(crate) trial: bool,
    pub(crate) shared: Arc<SessionShared>,
}

/// The audio of one recording on its way to VAD, through RNNoise if that's on.
struct Incoming<'a> {
    heard: &'a Receiver<Audio>,
    denoiser: Option<Denoiser>,
    /// Frames the denoiser finished, waiting to be handed out.
    ready: VecDeque<Vec<f32>>,
}

impl Incoming<'_> {
    /// The next audio. An error means the recording was abandoned.
    fn next(&mut self) -> Result<Audio, RecvError> {
        loop {
            if let Some(frame) = self.ready.pop_front() {
                return Ok(Audio::Frame(frame));
            }
            let audio = self.heard.recv()?;
            match (&mut self.denoiser, audio) {
                (Some(denoiser), Audio::Frame(frame)) => {
                    self.ready.extend(denoiser.process(&frame));
                }
                (_, audio) => return Ok(audio),
            }
        }
    }
}

/// What came out of the engines for one recording.
struct Heard {
    transcript: Transcript,
    released: Instant,
    duration: Duration,
    vad: Duration,
    asr: Duration,
}

impl Session {
    pub(crate) fn spawn(self) {
        thread::Builder::new()
            .name("wf-dictation-session".into())
            .spawn(move || self.run())
            .expect("the session thread starts");
    }

    fn run(self) {
        let context = self.read_context();
        match self.dictate(&context) {
            Ok(Some(insert)) if self.trial => {
                (self.shared.on_event)(Event::Trial(insert.text));
                self.shared.finished(self.id, None);
            }
            Ok(Some(insert)) => {
                let _ = self.shared.jobs.send(Job::Insert(Box::new(insert)));
            }
            Ok(None) => self.shared.finished(self.id, None),
            Err(problem) => self.shared.finished(self.id, Some(problem)),
        }
    }

    fn read_context(&self) -> Context {
        if self.settings.privacy.context_awareness {
            self.shared.context.read(ContextDepth::WithText)
        } else {
            Context {
                app: None,
                window_title: None,
                focus: Focus::Unknown,
                text: None,
            }
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    fn lock_engines(&self) -> MutexGuard<'_, Loaded> {
        self.shared
            .loaded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs the engines. `None` means nothing to insert: cancelled, or nothing was said.
    fn dictate(&self, context: &Context) -> Result<Option<Insert>, String> {
        // One dictation at a time on the engines. A later one records in the meantime.
        let mut loaded = self.lock_engines();
        let Ready { vad, asr } = loaded
            .prepare(&self.plan, &self.shared.engines)
            .map_err(|error| describe("load the models", &error))?;
        let options = AsrOptions {
            language: self.language(),
            bias_words: bias_words(&self.settings.style.dictionary, context.text.as_ref()),
        };
        let heard = match vad {
            Some(vad) => self.stream(vad, asr, options, &mut loaded)?,
            None => self.whole(asr, options, &mut loaded)?,
        };
        let Some(heard) = heard else {
            return Ok(None);
        };

        let started = Instant::now();
        let language = heard.transcript.language.as_deref();
        let style = &self.settings.style;
        let category = category_of(context);
        let chosen_style = style_for(style, category);
        let cleaned = rule_cleanup(&heard.transcript.text, language);
        let mut cleanup_fallback = None;
        let text = match loaded.llm() {
            Some(mut host) => {
                let request = CleanupRequest {
                    text: &cleaned,
                    language,
                    strength: self.settings.pipeline.cleanup_strength,
                    style: Some(chosen_style),
                    dictionary: &style.dictionary,
                };
                let outcome = llm_cleanup(&mut host, &request, DEFAULT_TIMEOUT);
                cleanup_fallback = outcome.fallback.as_ref().and_then(describe_fallback);
                outcome.text
            }
            None => cleaned.clone(),
        };
        drop(loaded);
        if self.cancelled() {
            return Ok(None);
        }

        let cleanup = started.elapsed();
        let started = Instant::now();
        let around = context.text.as_ref();
        let output = local_passes(
            &text,
            &LocalPassOptions {
                style: chosen_style,
                category,
                snippets: &style.snippets,
                replacements: &style.replacements,
                press_enter_command: style.press_enter_command,
                before: around.map(|text| text.before.as_str()),
                after: around.map(|text| text.after.as_str()),
            },
        );
        Ok(Some(Insert {
            id: self.id,
            text: output.text,
            press_enter: output.press_enter,
            raw_text: heard.transcript.text,
            app: context.app.as_ref().map(|app| app.id.clone()),
            duration: heard.duration,
            timings: Timings {
                vad: heard.vad,
                asr: heard.asr,
                cleanup,
                passes: started.elapsed(),
                insertion: Duration::ZERO,
            },
            released: heard.released,
            cleanup_fallback,
            problem: self.plan.llm_problem.clone(),
            cancel: Arc::clone(&self.cancel),
            paste_hint: crate::worker::paste_hint(&self.settings),
        }))
    }

    fn incoming(&self) -> Result<Incoming<'_>, String> {
        let denoiser = self
            .settings
            .pipeline
            .denoise
            .then(Denoiser::new)
            .transpose()
            .map_err(|error| format!("Couldn't start noise suppression: {error}"))?;
        Ok(Incoming {
            heard: &self.heard,
            denoiser,
            ready: VecDeque::new(),
        })
    }

    fn language(&self) -> Option<String> {
        match self.settings.general.languages.as_slice() {
            [only] => Some(only.clone()),
            _ => None,
        }
    }

    /// VAD on: chunks reach ASR as they close.
    fn stream(
        &self,
        mut vad: Box<dyn Vad>,
        asr: Box<dyn Asr>,
        options: AsrOptions,
        loaded: &mut Loaded,
    ) -> Result<Option<Heard>, String> {
        let auto_stop =
            self.settings.pipeline.vad_auto_stop.then(|| {
                Duration::from_secs(u64::from(self.settings.pipeline.vad_auto_stop_seconds))
            });
        let vad_options = VadOptions {
            auto_stop,
            ..VadOptions::default()
        };
        let worker = AsrWorker::<()>::spawn(asr, options);
        let mut live = LiveVad::new(vad.as_mut(), vad_options);
        let mut recording = Recording::default();
        let mut vad_time = Duration::ZERO;
        let mut auto_stopped = false;
        let failed = |error: PipelineError| describe("detect speech", &error);
        let mut incoming = self.incoming()?;
        let released = loop {
            match incoming.next() {
                Ok(Audio::Frame(frame)) => {
                    recording.note(&frame);
                    let started = Instant::now();
                    let update = live.push(&frame).map_err(failed)?;
                    vad_time += started.elapsed();
                    for chunk in update.chunks {
                        worker
                            .submit((), chunk)
                            .map_err(|error| describe("transcribe", &error))?;
                    }
                    if update.auto_stop && !auto_stopped && self.locked.load(Ordering::Relaxed) {
                        auto_stopped = true;
                        let _ = self.shared.inbox.send(Input::AutoStop(self.id));
                    }
                }
                Ok(Audio::End { released }) => break released,
                // Cancelled: the pump is gone without an end.
                Err(_) => {
                    let _ = live.finish();
                    if let Ok(asr) = worker.cancel() {
                        loaded.put_back(&self.plan, Some(vad), asr);
                    }
                    return Ok(None);
                }
            }
        };

        let started = Instant::now();
        let rest = live.finish().map_err(failed)?;
        vad_time += started.elapsed();
        for chunk in rest {
            worker
                .submit((), chunk)
                .map_err(|error| describe("transcribe", &error))?;
        }
        let (results, asr) = worker
            .finish()
            .map_err(|error| describe("transcribe", &error))?;
        let asr_time = released.elapsed();
        let mut text = Vec::new();
        let mut language = None;
        for result in results {
            match result {
                Ok(chunk) => {
                    language = language.or(chunk.transcript.language);
                    text.push(chunk.transcript.text);
                }
                Err(_) if self.cancelled() => return Ok(None),
                Err(error) => return Err(describe("transcribe", &error)),
            }
        }
        loaded.put_back(&self.plan, Some(vad), asr);
        if self.cancelled() {
            return Ok(None);
        }
        recording.check_signal()?;
        if text.is_empty() {
            return Ok(None);
        }
        Ok(Some(Heard {
            transcript: Transcript {
                text: text.join(" "),
                language,
            },
            released,
            duration: recording.duration(),
            vad: vad_time,
            asr: asr_time,
        }))
    }

    /// VAD off: the whole recording goes to ASR at the end.
    fn whole(
        &self,
        mut asr: Box<dyn Asr>,
        options: AsrOptions,
        loaded: &mut Loaded,
    ) -> Result<Option<Heard>, String> {
        let mut recording = Recording::default();
        let mut samples = Vec::new();
        let mut incoming = self.incoming()?;
        let released = loop {
            match incoming.next() {
                Ok(Audio::Frame(frame)) => {
                    recording.note(&frame);
                    samples.extend(frame);
                }
                Ok(Audio::End { released }) => break released,
                Err(_) => {
                    loaded.put_back(&self.plan, None, asr);
                    return Ok(None);
                }
            }
        };
        recording.check_signal()?;
        let clip = AudioClip::new(samples);
        let chunks = speech_chunks(&clip, None, &VadOptions::default())
            .map_err(|error| describe("transcribe", &error))?;
        let transcript = match transcribe_speech(chunks, asr.as_mut(), &options) {
            Ok(transcript) => transcript,
            Err(_) if self.cancelled() => return Ok(None),
            Err(error) => return Err(describe("transcribe", &error)),
        };
        loaded.put_back(&self.plan, None, asr);
        Ok(transcript.map(|transcript| Heard {
            transcript,
            released,
            duration: recording.duration(),
            vad: Duration::ZERO,
            asr: released.elapsed(),
        }))
    }
}

/// What is known about the audio of a recording so far.
#[derive(Default)]
struct Recording {
    count: usize,
    signal: bool,
}

impl Recording {
    fn note(&mut self, frame: &[f32]) {
        self.count += frame.len();
        self.signal |= frame.iter().any(|&sample| sample != 0.0);
    }

    fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.count as f64 / f64::from(SAMPLE_RATE_HZ))
    }

    /// Pure digital zeros mean the OS is feeding silence, usually a denied permission.
    fn check_signal(&self) -> Result<(), String> {
        if self.count > 0 && !self.signal {
            return Err(SILENT_MIC.to_owned());
        }
        Ok(())
    }
}

fn describe(doing: &str, error: &PipelineError) -> String {
    format!("Couldn't {doing}: {error}")
}

fn describe_fallback(fallback: &Fallback) -> Option<String> {
    match fallback {
        Fallback::Disabled => None,
        other => Some(format!("{other:?}")),
    }
}

fn category_of(context: &Context) -> AppCategory {
    app_category(
        context.app.as_ref().map(|app| app.id.as_str()),
        context.window_title.as_deref(),
    )
}

fn style_for(style: &StyleSettings, category: AppCategory) -> Style {
    match category {
        AppCategory::PersonalMessages => style.personal_messages,
        AppCategory::WorkMessages => style.work_messages,
        AppCategory::Email => style.email,
        AppCategory::Other => style.other,
    }
}

/// The dictionary, and the on-screen words most likely to be names, for the decoder. Engines weigh
/// the end of the list most, so the dictionary goes last.
fn bias_words(dictionary: &[String], around: Option<&TextAround>) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    if let Some(around) = around {
        let before = around.before.split_whitespace().rev();
        let after = around.after.split_whitespace();
        for word in before.chain(after) {
            let word = word.trim_matches(|c: char| !c.is_alphanumeric());
            let notable = word.chars().count() >= 3
                && word.chars().any(|c| c.is_uppercase() || c.is_numeric());
            if notable && !words.iter().any(|known| known == word) {
                words.push(word.to_owned());
                if words.len() == MAX_CONTEXT_WORDS {
                    break;
                }
            }
        }
    }
    words.reverse();
    words.extend_from_slice(dictionary);
    words
}
