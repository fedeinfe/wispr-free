//! Fakes for every seam of the flow, and a [`Rig`] that runs the whole of it on them.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use tempfile::TempDir;
use wf_audio::CaptureEvent;
use wf_core::audio::{AudioClip, FRAME_MS, FRAME_SAMPLES};
use wf_core::pill::PillState;
use wf_core::settings::{KeepModels, PipelineTier, Settings};
use wf_core::shortcut::{Bindings, Key, KeyEvent, Platform};
use wf_dictation::plan::{AsrModel, DiarizerModel, LlmModel};
use wf_dictation::{
    Clock, Dictation, Engines, Event, Insertion, MeetingRoom, MeetingTape, MicStream, Microphone,
    Parts,
};
use wf_meetings::{MeetingEvent, MeetingWriter, MicClient, StereoFrame, TimelineFrame};
use wf_models::Catalog;
use wf_pipeline::{
    Asr, AsrOptions, Diarizer, Llm, LlmPrompt, PipelineError, SpeakerTurn, SpeechSegment,
    Transcript, Vad,
};
use wf_platform::context::{AppIdentity, Context, ContextDepth, ContextSource, Focus, TextAround};
use wf_platform::insert::{CopyReason, InsertError, InsertOutcome, PasteChord};
use wf_platform::keys::KeySignal;
use wf_platform::media::{MediaControl, MediaError, Output};

pub const FN: Key = Key::Fn;
const WAIT: Duration = Duration::from_secs(10);

/// Reads a committed 16-bit mono WAV fixture from the pipeline crate.
pub fn fixture(name: &str) -> AudioClip {
    let path = format!(
        "{}/../pipeline/tests/fixtures/{name}.wav",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = fs::read(&path).expect(&path);
    let mut at = 12;
    loop {
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        if &bytes[at..at + 4] == b"data" {
            let samples = bytes[at + 8..at + 8 + size]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&pair| f32::from(i16::from_le_bytes(pair)) / 32_768.0)
                .collect();
            return AudioClip::new(samples);
        }
        at += 8 + size + size % 2;
    }
}

#[derive(Default)]
pub struct FakeClock(Mutex<Duration>);

impl FakeClock {
    pub fn advance(&self, by: Duration) {
        *self.0.lock().unwrap() += by;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Duration {
        *self.0.lock().unwrap()
    }
}

#[derive(Default)]
pub struct FakeMic {
    feed: Mutex<Option<Sender<CaptureEvent>>>,
    pub opened: AtomicUsize,
    /// Opens a test has already waited for.
    seen: AtomicUsize,
    pub open_now: Arc<AtomicUsize>,
    pub problem: Mutex<Option<String>>,
    pub ranking: Mutex<Vec<String>>,
}

impl FakeMic {
    pub fn send(&self, event: CaptureEvent) {
        let feed = self.feed.lock().unwrap();
        let _ = feed.as_ref().expect("the mic is open").send(event);
    }

    pub fn play(&self, clip: &AudioClip) {
        for frame in clip.samples().chunks(FRAME_SAMPLES) {
            let mut frame = frame.to_vec();
            frame.resize(FRAME_SAMPLES, 0.0);
            self.send(CaptureEvent::Frame(frame));
        }
    }

    /// Waits for the next open.
    pub fn wait_open(&self) {
        let next = self.seen.fetch_add(1, Ordering::SeqCst) + 1;
        wait_for("the mic to open", || {
            self.opened.load(Ordering::SeqCst) >= next
        });
    }

    pub fn wait_closed(&self) {
        wait_for("the mic to close", || {
            self.open_now.load(Ordering::SeqCst) == 0
        });
    }
}

struct FakeStream {
    events: Receiver<CaptureEvent>,
    open_now: Arc<AtomicUsize>,
}

impl MicStream for FakeStream {
    fn events(&self) -> &Receiver<CaptureEvent> {
        &self.events
    }
}

impl Drop for FakeStream {
    fn drop(&mut self) {
        self.open_now.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Microphone for FakeMic {
    fn problem(&self) -> Option<String> {
        self.problem.lock().unwrap().clone()
    }

    fn open(&self, ranking: Vec<String>) -> Result<Box<dyn MicStream>, String> {
        let (feed, events) = mpsc::channel();
        *self.feed.lock().unwrap() = Some(feed);
        *self.ranking.lock().unwrap() = ranking;
        self.opened.fetch_add(1, Ordering::SeqCst);
        self.open_now.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FakeStream {
            events,
            open_now: Arc::clone(&self.open_now),
        }))
    }
}

/// Calls frames speech by level, like a real detector but with no model. A segment closes after
/// half a second of quiet.
struct FakeVad {
    probe: Arc<Probe>,
    heard: usize,
    open: Option<SpeechSegment>,
    quiet: usize,
}

const QUIET_SAMPLES: usize = 8_000;
const SPEECH_LEVEL: f32 = 0.01;

impl Vad for FakeVad {
    fn accept(&mut self, samples: &[f32]) -> Result<Vec<SpeechSegment>, PipelineError> {
        let mut closed = Vec::new();
        for window in samples.chunks(512) {
            let start = self.heard;
            self.heard += window.len();
            let peak = window.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
            if peak >= SPEECH_LEVEL {
                self.quiet = 0;
                let open = self.open.get_or_insert(SpeechSegment { start, end: start });
                open.end = self.heard;
            } else if self.open.is_some() {
                self.quiet += window.len();
                if self.quiet >= QUIET_SAMPLES {
                    closed.extend(self.open.take());
                }
            }
        }
        Ok(closed)
    }

    fn is_speaking(&self) -> bool {
        self.open.is_some()
    }

    fn finish(&mut self) -> Result<Vec<SpeechSegment>, PipelineError> {
        self.heard = 0;
        self.quiet = 0;
        Ok(self.open.take().into_iter().collect())
    }
}

impl Drop for FakeVad {
    fn drop(&mut self) {
        self.probe.vad_alive.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Holds an engine in a call until [`Gate::open`] or an interrupt.
#[derive(Default)]
pub struct Gate {
    closed: Mutex<bool>,
    changed: Condvar,
    pub entered: AtomicBool,
}

impl Gate {
    pub fn close(&self) {
        *self.closed.lock().unwrap() = true;
    }

    pub fn open(&self) {
        *self.closed.lock().unwrap() = false;
        self.changed.notify_all();
    }

    fn pass(&self) {
        self.entered.store(true, Ordering::SeqCst);
        let mut closed = self.closed.lock().unwrap();
        while *closed {
            closed = self.changed.wait(closed).unwrap();
        }
    }
}

/// What the fake engines saw.
#[derive(Default)]
pub struct Probe {
    pub vad_loads: AtomicUsize,
    pub asr_loads: AtomicUsize,
    pub llm_loads: AtomicUsize,
    pub diarizer_loads: AtomicUsize,
    pub vad_alive: AtomicUsize,
    pub asr_alive: AtomicUsize,
    pub llm_alive: AtomicUsize,
    pub diarizer_alive: AtomicUsize,
    pub interrupts: AtomicUsize,
    /// Interrupts of anything but the dictation's own engines.
    pub scoped_interrupts: AtomicUsize,
    pub asr_model: Mutex<Option<AsrModel>>,
    pub llm_model: Mutex<Option<LlmModel>>,
    pub clips: Mutex<Vec<AudioClip>>,
    pub asr_options: Mutex<Option<AsrOptions>>,
    /// What ASR "hears", one reply per chunk, last one repeating.
    pub transcripts: Mutex<VecDeque<String>>,
    /// `None` makes the LLM fail.
    pub llm_reply: Mutex<Option<String>>,
    pub llm_prompts: Mutex<Vec<String>>,
    pub asr_gate: Gate,
    pub llm_gate: Gate,
    pub diarizer_gate: Gate,
    pub fail_asr_load: AtomicBool,
    pub fail_diarizer: AtomicBool,
    /// Answers the summary's prompts with what they ask for.
    pub writes_summaries: AtomicBool,
}

impl Probe {
    pub fn total_loads(&self) -> usize {
        self.vad_loads.load(Ordering::SeqCst)
            + self.asr_loads.load(Ordering::SeqCst)
            + self.llm_loads.load(Ordering::SeqCst)
    }

    pub fn total_alive(&self) -> usize {
        self.vad_alive.load(Ordering::SeqCst)
            + self.asr_alive.load(Ordering::SeqCst)
            + self.llm_alive.load(Ordering::SeqCst)
    }

    pub fn hears(&self, text: &str) {
        *self.transcripts.lock().unwrap() = VecDeque::from([text.to_owned()]);
    }

    pub fn llm_says(&self, text: &str) {
        *self.llm_reply.lock().unwrap() = Some(text.to_owned());
    }

    pub fn heard_samples(&self) -> usize {
        let clips = self.clips.lock().unwrap();
        clips.iter().map(|clip| clip.samples().len()).sum()
    }
}

struct FakeAsr {
    probe: Arc<Probe>,
}

impl Asr for FakeAsr {
    fn transcribe(
        &mut self,
        clip: &AudioClip,
        options: &AsrOptions,
    ) -> Result<Transcript, PipelineError> {
        self.probe.clips.lock().unwrap().push(clip.clone());
        *self.probe.asr_options.lock().unwrap() = Some(options.clone());
        self.probe.asr_gate.pass();
        if self.probe.interrupts.load(Ordering::SeqCst) > 0 {
            return Err(PipelineError::Engine("interrupted".into()));
        }
        let mut replies = self.probe.transcripts.lock().unwrap();
        let text = if replies.len() > 1 {
            replies.pop_front().unwrap()
        } else {
            replies.front().cloned().unwrap_or_default()
        };
        Ok(Transcript {
            text,
            language: Some("en".into()),
        })
    }
}

impl Drop for FakeAsr {
    fn drop(&mut self) {
        self.probe.asr_alive.fetch_sub(1, Ordering::SeqCst);
    }
}

struct FakeLlm {
    probe: Arc<Probe>,
}

impl Llm for FakeLlm {
    fn generate(
        &mut self,
        prompt: &LlmPrompt,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<String, PipelineError> {
        self.probe
            .llm_prompts
            .lock()
            .unwrap()
            .push(format!("{}\n{}", prompt.system, prompt.user));
        on_token("streaming ");
        let interrupts = self.probe.scoped_interrupts.load(Ordering::SeqCst);
        self.probe.llm_gate.pass();
        if self.probe.scoped_interrupts.load(Ordering::SeqCst) > interrupts {
            return Err(PipelineError::Engine("the model was killed".into()));
        }
        if self.probe.writes_summaries.load(Ordering::SeqCst) {
            let notes = "KEY POINTS\n- we ship on friday\nDECISIONS\n- none\nACTIONS\n- none";
            if prompt.system.contains("TITLE:") {
                return Ok("TITLE: The launch\nTLDR: we ship on friday".to_owned());
            }
            if prompt.system.contains("KEY POINTS") {
                return Ok(notes.to_owned());
            }
        }
        let reply = self.probe.llm_reply.lock().unwrap().clone();
        reply.ok_or_else(|| PipelineError::Engine("the model fell over".into()))
    }
}

impl Drop for FakeLlm {
    fn drop(&mut self) {
        self.probe.llm_alive.fetch_sub(1, Ordering::SeqCst);
    }
}

/// One voice all the way through.
struct FakeDiarizer {
    probe: Arc<Probe>,
}

impl Diarizer for FakeDiarizer {
    fn diarize(&mut self, clip: &AudioClip) -> Result<Vec<SpeakerTurn>, PipelineError> {
        let interrupts = self.probe.scoped_interrupts.load(Ordering::SeqCst);
        self.probe.diarizer_gate.pass();
        if self.probe.scoped_interrupts.load(Ordering::SeqCst) > interrupts {
            return Err(PipelineError::Engine("the speaker engine stopped".into()));
        }
        if self.probe.fail_diarizer.load(Ordering::SeqCst) {
            return Err(PipelineError::Engine("the speakers got mixed up".into()));
        }
        Ok(vec![SpeakerTurn {
            start: Duration::ZERO,
            end: clip.duration(),
            speaker: 0,
        }])
    }
}

impl Drop for FakeDiarizer {
    fn drop(&mut self) {
        self.probe.diarizer_alive.fetch_sub(1, Ordering::SeqCst);
    }
}

/// `false` for the engines of a dictation, `true` for every scope cut from them.
pub struct FakeEngines(pub Arc<Probe>, pub bool);

impl Engines for FakeEngines {
    fn vad(&self, _model: &Path) -> Result<Box<dyn Vad>, PipelineError> {
        self.0.vad_loads.fetch_add(1, Ordering::SeqCst);
        self.0.vad_alive.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FakeVad {
            probe: Arc::clone(&self.0),
            heard: 0,
            open: None,
            quiet: 0,
        }))
    }

    fn asr(&self, model: &AsrModel) -> Result<Box<dyn Asr>, PipelineError> {
        if self.0.fail_asr_load.load(Ordering::SeqCst) {
            return Err(PipelineError::Engine("the model file is damaged".into()));
        }
        *self.0.asr_model.lock().unwrap() = Some(model.clone());
        self.0.asr_loads.fetch_add(1, Ordering::SeqCst);
        self.0.asr_alive.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FakeAsr {
            probe: Arc::clone(&self.0),
        }))
    }

    fn llm(&self, model: &LlmModel) -> Result<Box<dyn Llm>, PipelineError> {
        *self.0.llm_model.lock().unwrap() = Some(model.clone());
        self.0.llm_loads.fetch_add(1, Ordering::SeqCst);
        self.0.llm_alive.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FakeLlm {
            probe: Arc::clone(&self.0),
        }))
    }

    fn diarizer(&self, _model: &DiarizerModel) -> Result<Box<dyn Diarizer>, PipelineError> {
        self.0.diarizer_loads.fetch_add(1, Ordering::SeqCst);
        self.0.diarizer_alive.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FakeDiarizer {
            probe: Arc::clone(&self.0),
        }))
    }

    fn interrupt(&self) {
        if self.1 {
            self.0.scoped_interrupts.fetch_add(1, Ordering::SeqCst);
            self.0.llm_gate.open();
            self.0.diarizer_gate.open();
        } else {
            self.0.interrupts.fetch_add(1, Ordering::SeqCst);
            self.0.asr_gate.open();
        }
    }

    fn scope(&self) -> Arc<dyn Engines> {
        Arc::new(Self(Arc::clone(&self.0), true))
    }
}

#[derive(Default)]
pub struct InsertLog {
    pub inserted: Mutex<Vec<String>>,
    pub copied: Mutex<Vec<String>>,
    pub enters: AtomicUsize,
    /// The next insert finds no text field.
    pub no_focus: AtomicBool,
    pub refused: AtomicBool,
}

struct FakeInsertion(Arc<InsertLog>);

impl Insertion for FakeInsertion {
    fn insert(
        &mut self,
        text: &str,
        on_pasted: &mut dyn FnMut(),
    ) -> Result<InsertOutcome, InsertError> {
        self.0.inserted.lock().unwrap().push(text.to_owned());
        if self.0.no_focus.load(Ordering::SeqCst) {
            return Ok(InsertOutcome::Copied {
                reason: CopyReason::NoFocusedField,
            });
        }
        if self.0.refused.load(Ordering::SeqCst) {
            return Ok(InsertOutcome::Copied {
                reason: CopyReason::PasteFailed("accessibility permission not granted".into()),
            });
        }
        on_pasted();
        Ok(InsertOutcome::Pasted {
            chord: PasteChord::CmdV,
            clipboard_restored: true,
        })
    }

    fn press_enter(&mut self) -> Result<(), InsertError> {
        self.0.enters.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn copy(&mut self, text: &str) -> Result<(), InsertError> {
        self.0.copied.lock().unwrap().push(text.to_owned());
        Ok(())
    }
}

type Writer = Arc<Mutex<Option<MeetingWriter>>>;
type Timeline = Arc<Mutex<Option<SyncSender<TimelineFrame>>>>;

/// Keeps what a meeting would have recorded out of reach of the real devices. The file is real,
/// because what follows the recording reads it.
#[derive(Default)]
pub struct FakeRoom {
    pub started: AtomicUsize,
    pub stopped: Arc<AtomicUsize>,
    pub paths: Mutex<Vec<PathBuf>>,
    pub refuse: AtomicBool,
    pub clients: Mutex<Vec<MicClient>>,
    pub playing: AtomicBool,
    feed: Mutex<Option<Sender<MeetingEvent>>>,
    timeline: Timeline,
    writer: Writer,
    frames: AtomicUsize,
}

impl FakeRoom {
    pub fn send(&self, event: MeetingEvent) {
        let feed = self.feed.lock().unwrap();
        feed.as_ref().expect("a meeting is on").send(event).unwrap();
    }

    /// Plays both channels into the file and the live transcript, as the recorder would.
    pub fn play(&self, you: &AudioClip, others: &AudioClip) {
        let padded = |clip: &AudioClip, at: usize| {
            let samples = clip.samples();
            let mut frame = samples[at.min(samples.len())..]
                .iter()
                .take(FRAME_SAMPLES)
                .copied()
                .collect::<Vec<_>>();
            frame.resize(FRAME_SAMPLES, 0.0);
            frame
        };
        let longest = you.samples().len().max(others.samples().len());
        for at in (0..longest).step_by(FRAME_SAMPLES) {
            let frame = StereoFrame {
                you: padded(you, at),
                others: padded(others, at),
            };
            let mut writer = self.writer.lock().unwrap();
            let writer = writer.as_mut().expect("a meeting is on");
            writer.write_frame(&frame.you, &frame.others).unwrap();
            let index = self.frames.fetch_add(1, Ordering::SeqCst) as u64;
            if let Some(timeline) = &*self.timeline.lock().unwrap() {
                let _ = timeline.send(TimelineFrame { index, frame });
            }
        }
    }

    pub fn played(&self) -> Duration {
        Duration::from_millis(self.frames.load(Ordering::SeqCst) as u64 * u64::from(FRAME_MS))
    }
}

struct FakeTape {
    events: Receiver<MeetingEvent>,
    stopped: Arc<AtomicUsize>,
    writer: Writer,
    timeline: Timeline,
}

impl MeetingTape for FakeTape {
    fn events(&self) -> &Receiver<MeetingEvent> {
        &self.events
    }

    fn stop(self: Box<Self>) -> Result<Duration, String> {
        self.stopped.fetch_add(1, Ordering::SeqCst);
        self.timeline.lock().unwrap().take();
        let writer = self.writer.lock().unwrap().take();
        writer
            .expect("a meeting is on")
            .finish()
            .map_err(|error| error.to_string())
    }
}

impl MeetingRoom for FakeRoom {
    fn record(
        &self,
        path: PathBuf,
        _ranking: Vec<String>,
        timeline: Option<SyncSender<TimelineFrame>>,
    ) -> Result<Box<dyn MeetingTape>, String> {
        if self.refuse.load(Ordering::SeqCst) {
            return Err("the disk said no".into());
        }
        let writer = MeetingWriter::create(&path).map_err(|error| error.to_string())?;
        *self.writer.lock().unwrap() = Some(writer);
        *self.timeline.lock().unwrap() = timeline;
        self.frames.store(0, Ordering::SeqCst);
        self.paths.lock().unwrap().push(path);
        self.started.fetch_add(1, Ordering::SeqCst);
        let (feed, events) = mpsc::channel();
        *self.feed.lock().unwrap() = Some(feed);
        Ok(Box::new(FakeTape {
            events,
            stopped: Arc::clone(&self.stopped),
            writer: Arc::clone(&self.writer),
            timeline: Arc::clone(&self.timeline),
        }))
    }

    fn mic_clients(&self) -> Vec<MicClient> {
        self.clients.lock().unwrap().clone()
    }

    fn other_app_playing(&self) -> bool {
        self.playing.load(Ordering::SeqCst)
    }
}

/// One system call of [`FakeMedia`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaCall {
    Mute(String),
    Unmute(String),
    Pause,
    Resume(Vec<String>),
    Ask,
}

/// The sound system, as far as the flow can tell, and what it was told to do.
pub struct FakeMedia {
    pub calls: Mutex<Vec<MediaCall>>,
    /// The default output. Anything else in `muted` is a device the user switched away from.
    pub output: Mutex<Result<String, MediaError>>,
    pub muted: Mutex<Vec<String>>,
    /// Devices that are unplugged.
    pub gone: Mutex<Vec<String>>,
    /// The players that are playing.
    pub playing: Mutex<Vec<String>>,
    /// Holds `output` and `pause_players`, the calls a slow system would be slow in.
    pub gate: Gate,
    /// Whether the crash note was on disk at the moment of each mute.
    pub noted_at_mute: Mutex<Vec<bool>>,
    pub note: Mutex<Option<PathBuf>>,
}

pub const SPEAKERS: &str = "BuiltInSpeakerDevice";

impl Default for FakeMedia {
    fn default() -> Self {
        Self {
            calls: Mutex::default(),
            output: Mutex::new(Ok(SPEAKERS.to_owned())),
            muted: Mutex::default(),
            gone: Mutex::default(),
            playing: Mutex::default(),
            gate: Gate::default(),
            noted_at_mute: Mutex::default(),
            note: Mutex::default(),
        }
    }
}

impl FakeMedia {
    pub fn calls(&self) -> Vec<MediaCall> {
        self.calls.lock().unwrap().clone()
    }

    pub fn is_muted(&self, device: &str) -> bool {
        self.muted.lock().unwrap().iter().any(|d| d == device)
    }

    pub fn wait_calls(&self, count: usize) {
        wait_for("calls to the sound system", || {
            self.calls.lock().unwrap().len() >= count
        });
    }
}

impl MediaControl for FakeMedia {
    fn output(&self) -> Result<Output, MediaError> {
        self.gate.pass();
        let device = self.output.lock().unwrap().clone()?;
        Ok(Output {
            muted: self.is_muted(&device),
            device,
        })
    }

    fn set_muted(&self, device: &str, muted: bool) -> Result<(), MediaError> {
        if self.gone.lock().unwrap().iter().any(|d| d == device) {
            return Err(MediaError::OutputGone);
        }
        let mut list = self.muted.lock().unwrap();
        let mut calls = self.calls.lock().unwrap();
        if muted {
            let note = self.note.lock().unwrap().clone();
            let noted = note.is_some_and(|note| note.exists());
            self.noted_at_mute.lock().unwrap().push(noted);
            list.push(device.to_owned());
            calls.push(MediaCall::Mute(device.to_owned()));
        } else {
            list.retain(|d| d != device);
            calls.push(MediaCall::Unmute(device.to_owned()));
        }
        Ok(())
    }

    fn pause_players(&self) -> Vec<String> {
        self.gate.pass();
        self.calls.lock().unwrap().push(MediaCall::Pause);
        std::mem::take(&mut *self.playing.lock().unwrap())
    }

    fn resume_players(&self, players: &[String]) {
        self.calls
            .lock()
            .unwrap()
            .push(MediaCall::Resume(players.to_vec()));
        self.playing.lock().unwrap().extend_from_slice(players);
    }

    fn ask_players(&self) {
        self.calls.lock().unwrap().push(MediaCall::Ask);
    }
}

pub struct FakeContext(pub Mutex<Context>);

impl ContextSource for FakeContext {
    fn read(&self, _depth: ContextDepth) -> Context {
        self.0.lock().unwrap().clone()
    }
}

pub fn editor_context() -> Context {
    Context {
        app: Some(AppIdentity {
            id: "com.example.Notes".into(),
            name: None,
            pid: None,
        }),
        window_title: None,
        focus: Focus::Editable,
        text: None,
    }
}

/// Every id the catalog's tiers name, with placeholder files of the right size.
pub fn install_models(root: &Path, catalog: &Catalog, ids: &[&str]) {
    for id in ids {
        let model = catalog.model(id).unwrap_or_else(|| panic!("no model {id}"));
        let dir = root.join(id);
        fs::create_dir_all(&dir).unwrap();
        for file in &model.files {
            let handle = fs::File::create(dir.join(&file.name)).unwrap();
            handle.set_len(file.size).unwrap();
        }
    }
}

pub const PARAKEET: &str = "parakeet-tdt-0.6b-v3-int8";
pub const WHISPER: &str = "whisper-small-q8_0";
pub const SILERO: &str = "silero-vad-v5";
pub const QWEN: &str = "qwen3-1.7b-q4_k_m";
pub const SEGMENTATION: &str = "pyannote-segmentation-3.0-int8";
pub const EMBEDDING: &str = "eres2net-voxceleb";

/// Rules only, on Parakeet with VAD, so most tests need no LLM.
pub fn base_settings() -> Settings {
    let mut settings = Settings {
        shortcuts: Bindings::default_for(Platform::MacOs),
        ..Settings::default()
    };
    settings.pipeline.tier = PipelineTier::Custom;
    settings.pipeline.asr_model = Some(PARAKEET.to_owned());
    settings.pipeline.vad_model = Some(SILERO.to_owned());
    settings.pipeline.llm_model = None;
    settings.pipeline.llm_cleanup = false;
    settings.pipeline.keep_models_loaded = KeepModels::Always;
    settings
}

/// One running flow, its fakes and everything it said.
pub struct Rig {
    pub dictation: Dictation,
    pub mic: Arc<FakeMic>,
    pub room: Arc<FakeRoom>,
    pub probe: Arc<Probe>,
    pub log: Arc<InsertLog>,
    pub clock: Arc<FakeClock>,
    pub context: Arc<FakeContext>,
    pub media: Arc<FakeMedia>,
    pub events: Receiver<Event>,
    pub data: TempDir,
}

pub const MEDIA_NOTE: &str = "media-note.json";

impl Rig {
    pub fn new(settings: Settings) -> Self {
        Self::build(
            settings,
            &[PARAKEET, WHISPER, SILERO, QWEN, SEGMENTATION, EMBEDDING],
        )
    }

    pub fn build(settings: Settings, installed: &[&str]) -> Self {
        let data = tempfile::tempdir().unwrap();
        let models = data.path().join("models");
        fs::create_dir_all(&models).unwrap();
        let catalog = Arc::new(Catalog::bundled());
        install_models(&models, &catalog, installed);
        Self::with_models(settings, data, models)
    }

    pub fn with_models(settings: Settings, data: TempDir, models: PathBuf) -> Self {
        Self::with_media(settings, data, models, Arc::default())
    }

    pub fn with_media(
        settings: Settings,
        data: TempDir,
        models: PathBuf,
        media: Arc<FakeMedia>,
    ) -> Self {
        let mic = Arc::new(FakeMic::default());
        let room = Arc::new(FakeRoom::default());
        let probe = Arc::new(Probe::default());
        let log = Arc::new(InsertLog::default());
        let clock = Arc::new(FakeClock::default());
        let context = Arc::new(FakeContext(Mutex::new(editor_context())));
        let (events_tx, events) = mpsc::channel();
        let events_tx = Mutex::new(events_tx);
        let insertion_log = Arc::clone(&log);
        let media_note = data.path().join(MEDIA_NOTE);
        *media.note.lock().unwrap() = Some(media_note.clone());
        let dictation = Dictation::start(Parts {
            settings,
            catalog: Arc::new(Catalog::bundled()),
            default_models: models,
            history: data.path().join("history.sqlite3"),
            meetings: data.path().join("meetings"),
            microphone: Arc::clone(&mic) as Arc<dyn Microphone>,
            meeting_room: Arc::clone(&room) as Arc<dyn MeetingRoom>,
            engines: Arc::new(FakeEngines(Arc::clone(&probe), false)),
            context: Arc::clone(&context) as Arc<dyn ContextSource + Send + Sync>,
            media: Arc::clone(&media) as Arc<dyn MediaControl>,
            media_note,
            insertion: Box::new(move || Box::new(FakeInsertion(insertion_log))),
            clock: Arc::clone(&clock) as Arc<dyn Clock>,
            on_event: Arc::new(move |event| {
                let _ = events_tx.lock().unwrap().send(event);
            }),
        });
        Self {
            dictation,
            mic,
            room,
            probe,
            log,
            clock,
            context,
            media,
            events,
            data,
        }
    }

    pub fn press(&self, key: Key) -> bool {
        self.dictation.on_key(KeySignal::Key(KeyEvent::press(key)))
    }

    pub fn release(&self, key: Key) -> bool {
        self.dictation
            .on_key(KeySignal::Key(KeyEvent::release(key)))
    }

    /// Moves time on, as if the shortcut were held that long.
    pub fn hold(&self, by: Duration) {
        self.clock.advance(by);
        self.dictation.poke();
    }

    /// Holds Fn, plays `clip`, and releases.
    pub fn dictate(&self, clip: &AudioClip) {
        self.press(FN);
        self.mic.wait_open();
        self.mic.play(clip);
        self.clock.advance(clip.duration());
        self.release(FN);
    }

    /// Waits for the event `pick` accepts, skipping the others.
    pub fn wait_event<T>(&self, what: &str, mut pick: impl FnMut(Event) -> Option<T>) -> T {
        let deadline = Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let event = self
                .events
                .recv_timeout(left)
                .unwrap_or_else(|_| panic!("no {what} in time"));
            if let Some(found) = pick(event) {
                return found;
            }
        }
    }

    pub fn wait_pill(&self, wanted: PillState) {
        self.wait_event(&format!("pill {wanted:?}"), |event| {
            (event == Event::Pill(wanted.clone())).then_some(())
        });
    }

    pub fn wait_error(&self) -> String {
        self.wait_event("an error", |event| match event {
            Event::Pill(PillState::Error { reason }) => Some(reason),
            _ => None,
        })
    }

    pub fn wait_toast(&self) -> String {
        self.wait_event("a toast", |event| match event {
            Event::Toast(text) => Some(text),
            _ => None,
        })
    }

    pub fn wait_inserted(&self) {
        self.wait_event("an insertion", |event| {
            matches!(event, Event::Inserted(_)).then_some(())
        });
    }

    pub fn inserted(&self) -> Vec<String> {
        self.log.inserted.lock().unwrap().clone()
    }

    /// Lets everything queued so far run through the flow.
    pub fn settle(&self) {
        std::thread::sleep(Duration::from_millis(60));
    }
}

pub fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

pub fn around(before: &str) -> TextAround {
    TextAround::new(before, "", "")
}
