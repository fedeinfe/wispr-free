//! A flow with nothing real under it: no microphone, no engine, no model, no paste. It runs the
//! whole of the real flow, so a check can dictate through `Shell` without touching the Mac.

use std::f32::consts::TAU;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::Duration;

use wf_audio::{CaptureEvent, InputDevice};
use wf_core::audio::{AudioClip, FRAME_MS, FRAME_SAMPLES};
use wf_core::settings::{KeepModels, PipelineTier, Settings};
use wf_core::shortcut::{Bindings, Platform};
use wf_dictation::plan::{AsrModel, DiarizerModel, LlmModel};
use wf_dictation::{
    Engines, Insertion, MeetingRoom, MeetingTape, MicStream, Microphone, Parts, SystemClock,
};
use wf_meetings::{MicClient, TimelineFrame};
use wf_models::Catalog;
use wf_pipeline::{Asr, AsrOptions, Diarizer, Llm, PipelineError, Transcript, Vad};
use wf_platform::context::{AppIdentity, Context, ContextDepth, ContextSource, Focus};
use wf_platform::insert::{InsertError, InsertOutcome, PasteChord};
use wf_platform::media::{MediaControl, MediaError, Output};

use crate::flow::OnEvent;
use crate::paths::Paths;
use crate::settings::{Event as PageEvent, Mailbox};

/// What every rehearsal hears.
pub const HEARD: &str = "This came through the whole flow";
/// The speech model a rehearsal pretends to have.
const SPEECH_MODEL: &str = "parakeet-tdt-0.6b-v3-int8";
/// Loud enough to show on the pill.
const SIGNAL: f32 = 0.2;
/// Whole cycles in a frame, so frames join up.
const TONE_CYCLES: f32 = 8.0;
const QUIET_BY: f32 = 30.0;
/// Frames between a rise and a fall.
const SYLLABLE_FRAMES: u32 = 8;

/// Rules only, on one speech model.
pub fn settings() -> Settings {
    let mut settings = Settings {
        shortcuts: Bindings::default_for(Platform::MacOs),
        ..Settings::default()
    };
    let pipeline = &mut settings.pipeline;
    pipeline.tier = PipelineTier::Custom;
    pipeline.asr_model = Some(SPEECH_MODEL.to_owned());
    pipeline.vad = false;
    pipeline.llm_cleanup = false;
    pipeline.keep_models_loaded = KeepModels::Always;
    settings
}

/// Makes up what the Settings pages learn from the machine, so a picture or a check sees rows on a
/// Mac with no microphone, and no real device name.
pub fn report_machine(mailbox: &Mailbox) {
    mailbox.post(PageEvent::Hardware {
        summary: "10 CPU cores · 16.0 GB RAM · Sample GPU (Metal)".to_owned(),
        recommended: PipelineTier::Balanced,
    });
    report_microphones(mailbox);
}

/// Onboarding looks again and again, and a rehearsal hears of no real device.
pub fn report_microphones(mailbox: &Mailbox) {
    let device = |id: &str, name: &str| InputDevice {
        id: id.to_owned(),
        name: name.to_owned(),
    };
    mailbox.post(PageEvent::Microphones {
        devices: vec![
            device("built-in", "Built-in Microphone"),
            device("usb", "USB Headset"),
        ],
        default: Some("built-in".to_owned()),
    });
}

/// Every seam of the flow on a fake, with its files at `paths`.
pub fn parts(paths: &Paths, settings: Settings, on_event: OnEvent) -> Parts {
    let catalog = Arc::new(Catalog::bundled());
    if let Err(error) = place_model(&paths.models, &catalog) {
        eprintln!("could not lay out the rehearsal's model: {error}");
    }
    Parts {
        settings,
        catalog,
        default_models: paths.models.clone(),
        media_note: paths.history.with_file_name("media-note.json"),
        history: paths.history.clone(),
        meetings: paths.meetings.clone(),
        microphone: Arc::new(ToneMicrophone),
        meeting_room: Arc::new(EmptyRoom),
        engines: Arc::new(ScriptedEngines),
        context: Arc::new(AnyField),
        media: Arc::new(QuietMedia),
        insertion: Box::new(|| Box::new(NowhereInsertion)),
        clock: Arc::new(SystemClock::default()),
        on_event,
    }
}

/// Placeholder files of the right size, which is all the flow looks at before it loads a model.
fn place_model(root: &Path, catalog: &Catalog) -> std::io::Result<()> {
    let Some(model) = catalog.model(SPEECH_MODEL) else {
        return Ok(());
    };
    let folder: PathBuf = root.join(SPEECH_MODEL);
    fs::create_dir_all(&folder)?;
    for file in &model.files {
        fs::File::create(folder.join(&file.name))?.set_len(file.size)?;
    }
    Ok(())
}

struct ToneMicrophone;

struct ToneStream(Receiver<CaptureEvent>);

impl MicStream for ToneStream {
    fn events(&self) -> &Receiver<CaptureEvent> {
        &self.0
    }
}

impl Microphone for ToneMicrophone {
    fn problem(&self) -> Option<String> {
        None
    }

    /// Plays in real time until the stream is dropped.
    fn open(&self, _ranking: Vec<String>) -> Result<Box<dyn MicStream>, String> {
        let (feed, events) = mpsc::sync_channel(64);
        thread::Builder::new()
            .name("rehearsal-mic".into())
            .spawn(move || tone(&feed))
            .map_err(|error| error.to_string())?;
        Ok(Box::new(ToneStream(events)))
    }
}

/// Rises and falls, as the level meter calls a steady sound its noise floor.
fn tone(feed: &SyncSender<CaptureEvent>) {
    let every = Duration::from_millis(u64::from(FRAME_MS));
    let loud: Vec<f32> = (0..FRAME_SAMPLES)
        .map(|sample| SIGNAL * (TAU * TONE_CYCLES * sample as f32 / FRAME_SAMPLES as f32).sin())
        .collect();
    let quiet: Vec<f32> = loud.iter().map(|sample| sample / QUIET_BY).collect();
    for frame in (0..).map(|frame| frame / SYLLABLE_FRAMES % 2) {
        let samples = if frame == 0 { &loud } else { &quiet };
        if feed.send(CaptureEvent::Frame(samples.clone())).is_err() {
            return;
        }
        thread::sleep(every);
    }
}

struct ScriptedEngines;

impl Engines for ScriptedEngines {
    fn vad(&self, _model: &Path) -> Result<Box<dyn Vad>, PipelineError> {
        Err(not_rehearsed())
    }

    fn asr(&self, _model: &AsrModel) -> Result<Box<dyn Asr>, PipelineError> {
        Ok(Box::new(ScriptedAsr))
    }

    fn llm(&self, _model: &LlmModel) -> Result<Box<dyn Llm>, PipelineError> {
        Err(not_rehearsed())
    }

    fn diarizer(&self, _model: &DiarizerModel) -> Result<Box<dyn Diarizer>, PipelineError> {
        Err(not_rehearsed())
    }

    fn interrupt(&self) {}

    fn scope(&self) -> Arc<dyn Engines> {
        Arc::new(Self)
    }
}

fn not_rehearsed() -> PipelineError {
    PipelineError::Engine("not part of a rehearsal".into())
}

struct ScriptedAsr;

impl Asr for ScriptedAsr {
    fn transcribe(
        &mut self,
        _clip: &AudioClip,
        _options: &AsrOptions,
    ) -> Result<Transcript, PipelineError> {
        Ok(Transcript {
            text: HEARD.to_owned(),
            language: Some("en".into()),
        })
    }
}

struct EmptyRoom;

impl MeetingRoom for EmptyRoom {
    fn record(
        &self,
        _path: PathBuf,
        _ranking: Vec<String>,
        _timeline: Option<SyncSender<TimelineFrame>>,
    ) -> Result<Box<dyn MeetingTape>, String> {
        Err("not part of a rehearsal".to_owned())
    }

    fn mic_clients(&self) -> Vec<MicClient> {
        Vec::new()
    }

    fn other_app_playing(&self) -> bool {
        false
    }
}

struct AnyField;

impl ContextSource for AnyField {
    fn read(&self, _depth: ContextDepth) -> Context {
        Context {
            app: Some(AppIdentity {
                id: "com.example.Rehearsal".into(),
                name: None,
                pid: None,
            }),
            window_title: None,
            focus: Focus::Editable,
            text: None,
        }
    }
}

struct QuietMedia;

impl MediaControl for QuietMedia {
    fn output(&self) -> Result<Output, MediaError> {
        Err(MediaError::NoOutput)
    }

    fn set_muted(&self, _device: &str, _muted: bool) -> Result<(), MediaError> {
        Err(MediaError::NoOutput)
    }

    fn pause_players(&self) -> Vec<String> {
        Vec::new()
    }

    fn resume_players(&self, _players: &[String]) {}

    fn ask_players(&self) {}
}

struct NowhereInsertion;

impl Insertion for NowhereInsertion {
    fn insert(
        &mut self,
        _text: &str,
        on_pasted: &mut dyn FnMut(),
    ) -> Result<InsertOutcome, InsertError> {
        on_pasted();
        Ok(InsertOutcome::Pasted {
            chord: PasteChord::CmdV,
            clipboard_restored: true,
        })
    }

    fn press_enter(&mut self) -> Result<(), InsertError> {
        Ok(())
    }

    fn copy(&mut self, _text: &str) -> Result<(), InsertError> {
        Ok(())
    }
}
