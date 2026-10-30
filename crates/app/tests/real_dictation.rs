//! The real engines in real child processes, from audio to the insertion seam. Skipped by default:
//!
//! `WF_MODELS=…/wispr-free-notes/models cargo test -p wispr-free --release --test real_dictation -- --ignored --nocapture --test-threads=1`
//!
//! The spec's two sentences have no committed recording, so these tests have macOS's `say` speak
//! them at run time, into a temp folder. Its voices can't be committed (see the fixtures README).
#![cfg(target_os = "macos")]

use std::collections::HashSet;
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
use wf_dictation::plan::preset_tier;
use wf_dictation::probe::{ClipMicrophone, NoContext, TextLog, read_wav};
use wf_dictation::remote::ChildEngines;
use wf_dictation::{Dictation, Event, Parts, Report, SystemClock, SystemMeetingRoom};
use wf_models::Catalog;

const LONG_WAIT: Duration = Duration::from_secs(60);
/// Spec §8's sentence. The commas make the voice pause the way a person does; run together, both
/// ASR engines hear "I'm so" from it, as they would from anyone who says it that fast.
const FILLERS: &str = "Um, so I, I think we should, uh, ship it.";
const CORRECTION: &str = "I like the red one, oh no, I meant the blue one";

/// A model folder laid out the way the app expects, made of links to the test models.
fn link_models(root: &Path) {
    let source = PathBuf::from(std::env::var("WF_MODELS").expect("set WF_MODELS"));
    for model in Catalog::bundled().models {
        let folder = source.join(&model.id);
        let loose = source.join(&model.files[0].name);
        if folder.is_dir() {
            symlink(folder, root.join(&model.id)).unwrap();
        } else if loose.is_file() {
            let dir = root.join(&model.id);
            fs::create_dir_all(&dir).unwrap();
            symlink(loose, dir.join(&model.files[0].name)).unwrap();
        }
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
        meetings: data.path().join("meetings"),
        microphone: Arc::clone(&microphone) as _,
        meeting_room: Arc::new(SystemMeetingRoom),
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
        self.try_dictate(clip)
            .unwrap_or_else(|reason| panic!("{reason}"))
    }

    /// [`Self::dictate`], with the pill's error for a dictation that failed.
    fn try_dictate(&self, clip: AudioClip) -> Result<(String, Report), String> {
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
                Event::Pill(PillState::Error { reason }) => return Err(reason),
                _ => {}
            }
        };
        let text = self.log.texts().pop().expect("text was inserted");
        Ok((text, report))
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
fn fillers_come_out_clean_on_high() {
    let flow = start(tier(PipelineTier::High), false);
    let clip = spoken(FILLERS, flow.data.path());

    let (text, report) = flow.dictate(with_pauses(&clip));

    println!("high: {text:?} {report:?}");
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
fn noise_suppression_leaves_clean_speech_readable() {
    let mut settings = tier(PipelineTier::Balanced);
    settings.pipeline.denoise = true;
    let flow = start(settings, false);
    let folder = tempfile::tempdir().unwrap();

    let lead_in = AudioClip::new(vec![0.000_1; 64_000]);
    let (text, _) = flow.dictate(with_pauses(&joined(
        &lead_in,
        &spoken(FILLERS, folder.path()),
    )));

    assert_eq!(text, "So I think we should ship it.");
}

/// `first`, then `second`.
fn joined(first: &AudioClip, second: &AudioClip) -> AudioClip {
    AudioClip::new([first.samples(), second.samples()].concat())
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
fn a_self_correction_resolves_on_high() {
    let flow = start(tier(PipelineTier::High), false);
    let clip = spoken(CORRECTION, flow.data.path());

    let (text, report) = flow.dictate(with_pauses(&clip));

    println!("high: {text:?} {report:?}");
    assert_eq!(text, "I like the blue one.");
}

#[test]
#[ignore = "needs models"]
fn latency_of_a_ten_second_clip() {
    let clip = fixture("speech_mid_phrase_en");
    let ten_seconds = AudioClip::new(clip.samples()[..160_000].to_vec());
    for (name, settings) in [
        ("high", tier(PipelineTier::High)),
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

/// One setup on one clip, for `docs/benchmarks.md`: a cold dictation, then warm ones. Reads
/// `WF_BENCH_CLIP` (a WAV), `WF_BENCH_TIER` and `WF_BENCH_RUNS`. `WF_BENCH_ASR` and `WF_BENCH_LLM`
/// try other models on that tier's stages, and `WF_BENCH_LANGUAGE` fills in "My languages".
#[test]
#[ignore = "needs models"]
fn tier_benchmark() {
    let env = |name: &str| std::env::var(name).ok();
    let name = env("WF_BENCH_TIER").expect("set WF_BENCH_TIER");
    let mut settings = tier(match name.as_str() {
        "lite" => PipelineTier::Lite,
        "balanced" => PipelineTier::Balanced,
        "high" => PipelineTier::High,
        other => panic!("unknown tier {other}"),
    });
    if env("WF_BENCH_ASR").is_some() || env("WF_BENCH_LLM").is_some() {
        let catalog = Catalog::bundled();
        let preset = catalog.preset(preset_tier(settings.pipeline.tier).unwrap());
        settings.pipeline.tier = PipelineTier::Custom;
        settings.pipeline.vad_model = preset.vad_model.clone();
        settings.pipeline.asr_model = env("WF_BENCH_ASR").or(Some(preset.asr_model.clone()));
        settings.pipeline.llm_model = env("WF_BENCH_LLM").or(preset.llm_model.clone());
    }
    settings.general.languages = env("WF_BENCH_LANGUAGE").into_iter().collect();
    let runs: usize = env("WF_BENCH_RUNS").map_or(6, |runs| runs.parse().unwrap());
    let clip = read_wav(Path::new(&env("WF_BENCH_CLIP").expect("set WF_BENCH_CLIP"))).unwrap();

    let flow = start(settings, true);
    for run in 0..runs {
        let (text, report) = flow.dictate(with_pauses(&clip));
        println!(
            "bench run={run} release_ms={} asr_ms={} cleanup_ms={} fallback={:?} text={text:?}",
            report.release_to_insert.as_millis(),
            report.timings.asr.as_millis(),
            report.timings.cleanup.as_millis(),
            report.cleanup_fallback,
        );
    }
}

/// What a late mic costs someone who talks the moment they press: the first `ms` of the speech
/// never arrive. The time itself comes from `cargo run -p wf-audio --example first_frame`.
#[test]
#[ignore = "needs models"]
fn what_each_late_millisecond_costs() {
    let folder = tempfile::tempdir().unwrap();
    let sentence = "Please review the budget before Friday.";
    let clip = spoken(sentence, folder.path());
    let onset = clip
        .samples()
        .iter()
        .position(|sample| sample.abs() > 0.01)
        .expect("the voice speaks");
    for (name, settings) in [
        ("balanced", tier(PipelineTier::Balanced)),
        ("lite", tier(PipelineTier::Lite)),
    ] {
        let flow = start(settings, false);
        for ms in [0, 50, 100, 150, 200, 300, 400, 500] {
            let cut = AudioClip::new(clip.samples()[onset + ms * 16..].to_vec());
            let (text, _) = flow.dictate(with_pauses(&cut));
            println!("{name} {ms} ms late: {text}");
        }
    }
}

/// The models on a real volume that goes away under loaded engines, the way an unplugged drive
/// does, and comes back. Uses a disk image in a temp folder, which Finder never shows.
#[test]
#[ignore = "needs models"]
fn an_unplugged_models_drive_is_an_error_and_plugging_it_back_recovers() {
    let folder = tempfile::tempdir().unwrap();
    let image = folder.path().join("models.sparseimage");
    let volume = folder.path().join("volume");
    let hdiutil = |args: &[&str]| {
        let status = Command::new("hdiutil").args(args).status().unwrap();
        assert!(status.success(), "hdiutil {args:?}");
    };
    let image_path = image.to_str().unwrap();
    let mount = volume.to_str().unwrap();
    let attach = [
        "attach",
        image_path,
        "-nobrowse",
        "-quiet",
        "-mountpoint",
        mount,
    ];
    let detach = ["detach", mount, "-force", "-quiet"];
    hdiutil(&[
        "create", "-quiet", "-size", "4g", "-fs", "APFS", "-type", "SPARSE", "-volname",
        "wfmodels", image_path,
    ]);
    fs::create_dir_all(&volume).unwrap();
    hdiutil(&attach);
    let links = folder.path().join("links");
    fs::create_dir_all(&links).unwrap();
    link_models(&links);
    let copied = Command::new("cp")
        .arg("-RL")
        .arg(&links)
        .arg(volume.join("models"))
        .status()
        .unwrap();
    assert!(copied.success());

    let mut settings = tier(PipelineTier::Balanced);
    settings.pipeline.models_folder = Some(volume.join("models"));
    let flow = start(settings, false);
    let clip = with_pauses(&fixture("speech_en"));

    let (text, _) = flow.dictate(clip.clone());
    assert!(!text.is_empty());
    let hosts = engine_hosts();

    hdiutil(&detach);
    flow.dictation.on_pill(PillInput::Click);
    let reason = loop {
        let event = flow.events.recv_timeout(LONG_WAIT).expect("no error shown");
        if let Event::Pill(PillState::Error { reason }) = event {
            break reason;
        }
    };
    println!("unplugged: {reason}");
    assert_eq!(engine_hosts(), hosts, "no engine died");

    hdiutil(&attach);
    let (text, _) = flow.dictate(clip);
    assert!(!text.is_empty());
    hdiutil(&detach);
}

/// ADR-0005: a dead host fails the next call made to it, and the dictation after that gets a new
/// one. With all three gone that takes two dictations: VAD fails the first before the LLM is
/// reached, and the second finds the dead LLM and falls back to rule-cleaned text.
#[test]
#[ignore = "needs models"]
fn killed_engine_hosts_are_replaced_by_the_dictations_that_need_them() {
    let flow = start(tier(PipelineTier::Balanced), false);
    let clip = with_pauses(&fixture("speech_en"));
    let (expected, _) = flow.dictate(clip.clone());

    let killed = Command::new("pkill")
        .args(["-9", "-P", &std::process::id().to_string()])
        .status()
        .unwrap();
    assert!(killed.success());

    let reason = flow.try_dictate(clip.clone()).map(|_| ()).unwrap_err();
    assert!(reason.contains("stopped"), "{reason}");
    std::thread::sleep(Duration::from_secs(1));
    let _ = flow.events.try_iter().count();

    let (_, report) = flow.dictate(clip.clone());
    assert!(report.cleanup_fallback.is_some());

    let (text, report) = flow.dictate(clip);
    assert_eq!(report.cleanup_fallback, None);
    assert_eq!(text, expected);
}

/// Ticket 07 asked for one long clip before Parakeet may run without VAD: at that point nothing
/// splits the recording at pauses, and a transducer's memory grows with its input.
#[test]
#[ignore = "needs models"]
fn parakeet_keeps_up_with_a_ten_minute_clip_without_vad() {
    let script = long_script(1_750);
    let folder = tempfile::tempdir().unwrap();
    let clip = spoken(&script, folder.path());
    let minutes = clip.duration().as_secs_f32() / 60.0;

    let mut settings = tier(PipelineTier::Lite);
    settings.pipeline.vad = false;
    let flow = start(settings, false);
    let (text, report) = flow.dictate(clip);

    let heard = |text: &str| -> HashSet<String> { words(text).into_iter().collect() };
    let said = words(&script);
    let known = heard(&text);
    let recall = said.iter().filter(|word| known.contains(*word)).count() as f32;
    let recall = recall / said.len() as f32;
    let tail = said
        .iter()
        .rev()
        .take(40)
        .filter(|word| known.contains(*word));
    println!(
        "{minutes:.1} min: release to paste {:?}, {} of {} words back, recall {recall:.3}",
        report.release_to_insert,
        words(&text).len(),
        said.len(),
    );
    assert!(recall > 0.95, "recall {recall}");
    assert!(tail.count() >= 36, "the end of the recording got lost");
}

/// About `count` words that don't repeat in order, the way nothing else in the fixtures does.
fn long_script(count: usize) -> String {
    const WHO: [&str; 6] = [
        "the engineer",
        "my sister",
        "our neighbour",
        "the driver",
        "a student",
        "the mayor",
    ];
    const DID: [&str; 6] = [
        "carried",
        "painted",
        "repaired",
        "described",
        "photographed",
        "ordered",
    ];
    const WHAT: [&str; 6] = [
        "a wooden table",
        "the broken window",
        "an old bicycle",
        "the yellow boat",
        "a small garden",
        "the kitchen door",
    ];
    const WHEN: [&str; 6] = [
        "before breakfast",
        "after the storm",
        "on Tuesday morning",
        "during the holiday",
        "late last night",
        "in early spring",
    ];
    let mut script = String::new();
    let mut seed = 7_usize;
    while script.split_whitespace().count() < count {
        let mut pick = |choices: &[&'static str; 6]| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            choices[(seed >> 16) % 6]
        };
        let (who, did, what, when) = (pick(&WHO), pick(&DID), pick(&WHAT), pick(&WHEN));
        script.push_str(&format!("{who} {did} {what} {when}. "));
    }
    script
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect()
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
