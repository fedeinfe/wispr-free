//! The real engines in real child processes, from audio to the insertion seam. Skipped by default:
//!
//! `WF_MODELS=…/wispr-free-notes/models cargo test -p wispr-free --release --test real_dictation -- --ignored --nocapture --test-threads=1`
//!
//! The spec's two sentences have no committed recording, so these tests have macOS's `say` speak
//! them at run time, into a temp folder. Its voices can't be committed (see the fixtures README).
#![cfg(target_os = "macos")]

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tempfile::TempDir;
use wf_core::audio::AudioClip;
use wf_core::dictation::PillInput;
use wf_core::pill::PillState;
use wf_core::settings::{KeepModels, PipelineTier, Settings};
use wf_dictation::probe::{ClipMicrophone, NoContext, TextLog, read_wav};
use wf_dictation::remote::ChildEngines;
use wf_dictation::{Dictation, Event, Parts, Report, SystemClock};
use wf_models::Catalog;

const LONG_WAIT: Duration = Duration::from_secs(60);
/// Spec §8's sentence. The commas make the voice pause the way a person does; run together, both
/// ASR engines hear "I'm so" from it, as they would from anyone who says it that fast.
const FILLERS: &str = "Um, so I, I think we should, uh, ship it.";
const CORRECTION: &str = "I like the red one, oh no, I meant the blue one";

/// A model folder laid out the way the app expects, made of links to the test models.
fn link_models(root: &Path) {
    let source = PathBuf::from(std::env::var("WF_MODELS").expect("set WF_MODELS"));
    let catalog = Catalog::bundled();
    let single_files = [
        ("silero-vad-v5", "silero_vad.onnx"),
        ("qwen3-1.7b-q4_k_m", "Qwen3-1.7B-Q4_K_M.gguf"),
    ];
    for (id, file) in single_files {
        let dir = root.join(id);
        fs::create_dir_all(&dir).unwrap();
        let name = &catalog.model(id).unwrap().files[0].name;
        symlink(source.join(file), dir.join(name)).unwrap();
    }
    for id in ["parakeet-tdt-0.6b-v3-int8", "whisper-small-q8_0"] {
        symlink(source.join(id), root.join(id)).unwrap();
    }
}

/// `say` reading `text`, at the pipeline's format.
fn spoken(text: &str, folder: &Path) -> AudioClip {
    let (aiff, wav) = (folder.join("spoken.aiff"), folder.join("spoken.wav"));
    let said = Command::new("say")
        .args(["-v", "Samantha", "-o"])
        .arg(&aiff)
        .arg(text)
        .status()
        .expect("say runs");
    assert!(said.success());
    let converted = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-i"])
        .arg(&aiff)
        .args(["-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le"])
        .arg(&wav)
        .status()
        .expect("ffmpeg runs");
    assert!(converted.success());
    read_wav(&wav).unwrap()
}

fn fixture(name: &str) -> AudioClip {
    let path = format!(
        "{}/../pipeline/tests/fixtures/{name}.wav",
        env!("CARGO_MANIFEST_DIR")
    );
    read_wav(Path::new(&path)).unwrap()
}

/// The padding a hold has around the words, so the detector sees a real start and end.
fn with_pauses(clip: &AudioClip) -> AudioClip {
    let pause = vec![0.0; 6_400];
    let samples = [pause.as_slice(), clip.samples(), &pause].concat();
    AudioClip::new(samples)
}

struct RealFlow {
    dictation: Dictation,
    microphone: Arc<ClipMicrophone>,
    log: TextLog,
    events: Receiver<Event>,
    data: TempDir,
}

fn start(settings: Settings, real_time: bool) -> RealFlow {
    let data = tempfile::tempdir().unwrap();
    let models = data.path().join("models");
    fs::create_dir_all(&models).unwrap();
    link_models(&models);

    let microphone = Arc::new(ClipMicrophone::new(AudioClip::default(), real_time));
    let log = TextLog::default();
    let insertion = log.clone();
    let (events_tx, events) = mpsc::channel();
    let events_tx = Mutex::new(events_tx);
    let dictation = Dictation::start(Parts {
        settings,
        catalog: Arc::new(Catalog::bundled()),
        default_models: models,
        history: data.path().join("history.sqlite3"),
        microphone: Arc::clone(&microphone) as _,
        engines: Arc::new(ChildEngines::new(PathBuf::from(env!(
            "CARGO_BIN_EXE_wispr-free"
        )))),
        context: Arc::new(NoContext),
        insertion: Box::new(move || Box::new(insertion)),
        clock: Arc::new(SystemClock::default()),
        on_event: Arc::new(move |event| {
            let _ = events_tx.lock().unwrap().send(event);
        }),
    });
    RealFlow {
        dictation,
        microphone,
        log,
        events,
        data,
    }
}

impl RealFlow {
    /// One hands-free dictation of `clip`. Returns what would have been typed.
    fn dictate(&self, clip: AudioClip) -> (String, Report) {
        self.microphone.set_clip(clip);
        self.dictation.on_pill(PillInput::Click);
        let give_up = Instant::now() + LONG_WAIT;
        while !self.microphone.played() {
            assert!(Instant::now() < give_up, "the clip never played");
            std::thread::sleep(Duration::from_millis(5));
        }
        self.dictation.on_pill(PillInput::Stop);
        let report = loop {
            let event = self.events.recv_timeout(LONG_WAIT).expect("no insertion");
            match event {
                Event::Inserted(report) => break report,
                Event::Pill(PillState::Error { reason }) => panic!("{reason}"),
                _ => {}
            }
        };
        let text = self.log.texts().pop().expect("text was inserted");
        (text, report)
    }

    fn wait_unloaded(&self) {
        loop {
            let event = self.events.recv_timeout(LONG_WAIT).expect("no unload");
            if event == Event::Unloaded {
                return;
            }
        }
    }
}

fn tier(tier: PipelineTier) -> Settings {
    let mut settings = Settings::default();
    settings.pipeline.tier = tier;
    settings.pipeline.llm_cleanup = tier != PipelineTier::Lite;
    settings.pipeline.keep_models_loaded = KeepModels::Always;
    settings
}

#[test]
#[ignore = "needs models"]
fn fillers_come_out_clean_on_balanced() {
    let flow = start(tier(PipelineTier::Balanced), false);
    let clip = spoken(FILLERS, flow.data.path());

    let (text, report) = flow.dictate(with_pauses(&clip));

    println!("balanced: {text:?} {report:?}");
    assert_eq!(text, "So I think we should ship it.");
}

#[test]
#[ignore = "needs models"]
fn fillers_come_out_clean_on_lite_with_the_llm_off() {
    let flow = start(tier(PipelineTier::Lite), false);
    let clip = spoken(FILLERS, flow.data.path());

    let (text, report) = flow.dictate(with_pauses(&clip));

    println!("lite: {text:?} {report:?}");
    assert_eq!(text, "So I think we should ship it.");
}

#[test]
#[ignore = "needs models"]
fn a_self_correction_resolves_on_balanced() {
    let flow = start(tier(PipelineTier::Balanced), false);
    let clip = spoken(CORRECTION, flow.data.path());

    let (text, report) = flow.dictate(with_pauses(&clip));

    println!("balanced: {text:?} {report:?}");
    assert_eq!(text, "I like the blue one.");
}

#[test]
#[ignore = "needs models"]
fn latency_of_a_ten_second_clip() {
    let clip = fixture("speech_mid_phrase_en");
    let ten_seconds = AudioClip::new(clip.samples()[..160_000].to_vec());
    for (name, settings) in [
        ("balanced", tier(PipelineTier::Balanced)),
        ("lite", tier(PipelineTier::Lite)),
    ] {
        let flow = start(settings, true);
        for run in ["cold", "warm", "warm"] {
            let (text, report) = flow.dictate(with_pauses(&ten_seconds));
            println!(
                "{name} {run}: release to paste {:?} (asr after release {:?}, cleanup {:?}, \
                 passes {:?}); {} chars; fallback {:?}",
                report.release_to_insert,
                report.timings.asr,
                report.timings.cleanup,
                report.timings.passes,
                text.len(),
                report.cleanup_fallback,
            );
        }
    }
}

#[test]
#[ignore = "needs models"]
fn unload_ends_every_engine_process() {
    let mut settings = tier(PipelineTier::Balanced);
    settings.pipeline.keep_models_loaded = KeepModels::UnloadAfterUse;
    let flow = start(settings, false);

    let (text, _) = flow.dictate(with_pauses(&fixture("speech_en")));
    assert!(!text.is_empty());
    flow.wait_unloaded();

    let give_up = Instant::now() + LONG_WAIT;
    while engine_hosts() > 0 {
        assert!(Instant::now() < give_up, "a host is still running");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Engine hosts started by this test process.
fn engine_hosts() -> usize {
    let list = Command::new("pgrep")
        .args(["-P", &std::process::id().to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&list.stdout).lines().count()
}
