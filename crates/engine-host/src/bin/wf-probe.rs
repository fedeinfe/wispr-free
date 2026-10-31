//! The dictation flow with a recording for a mic and a log for the focused app, so it can be
//! measured beside someone working: it opens no device, watches no keys and pastes nowhere. Each
//! `dictate` on stdin dictates the recording once and prints the text. `quit` or the end of stdin
//! quits. For development: bundles leave it out.
//!
//! `wf-probe --probe-clip=<16 kHz mono WAV> --data=<folder>`
//!
//! The folder holds all it reads and writes: `settings.toml`, `history.sqlite3`, `meetings` and
//! `models`, unless the settings name another models folder. Its hosts are the `wf-engine-host`
//! beside it, so run `cargo build -p wf-engine-host` first.

use std::env;
use std::io::{BufRead, stdin};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use wf_core::audio::AudioClip;
use wf_core::dictation::PillInput;
use wf_core::settings::SettingsStore;
use wf_dictation::probe::{ClipMicrophone, NoContext, NoMedia, TextLog, read_wav};
use wf_dictation::remote::engine_host_beside;
use wf_dictation::{Dictation, Parts};
use wf_models::Catalog;

const POLL: Duration = Duration::from_millis(5);

struct ClipRun {
    clip: AudioClip,
    microphone: Arc<ClipMicrophone>,
    log: TextLog,
    flow: Dictation,
}

impl ClipRun {
    fn start(clip: AudioClip, data: &Path) -> std::io::Result<Self> {
        let microphone = Arc::new(ClipMicrophone::new(clip.clone(), true));
        let log = TextLog::default();
        let insertion = log.clone();
        let mut parts = Parts::system(
            SettingsStore::open(data.join("settings.toml")).get(),
            Arc::new(Catalog::bundled()),
            data.join("models"),
            data.join("history.sqlite3"),
            data.join("meetings"),
            engine_host_beside(&env::current_exe()?),
            Arc::new(|_| {}),
        );
        parts.microphone = Arc::clone(&microphone) as _;
        parts.context = Arc::new(NoContext);
        parts.media = Arc::new(NoMedia);
        parts.insertion = Box::new(move || Box::new(insertion));
        Ok(Self {
            clip,
            microphone,
            log,
            flow: Dictation::start(parts),
        })
    }

    fn dictate(&self) -> String {
        let before = self.log.texts().len();
        self.microphone.set_clip(self.clip.clone());
        self.flow.on_pill(PillInput::Click);
        while !self.microphone.played() {
            thread::sleep(POLL);
        }
        self.flow.on_pill(PillInput::Stop);
        loop {
            if let Some(text) = self.log.texts().get(before) {
                return text.clone();
            }
            thread::sleep(POLL);
        }
    }
}

fn run(clip: &str, data: &str) -> Result<(), String> {
    let clip = read_wav(Path::new(clip))?;
    let run = ClipRun::start(clip, Path::new(data)).map_err(|error| error.to_string())?;
    println!("ready");
    for line in stdin().lock().lines().map_while(Result::ok) {
        match line.as_str() {
            "dictate" => println!("inserted: {}", run.dictate()),
            "quit" => break,
            _ => {}
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let flag = |name: &str| args.iter().find_map(|arg| arg.strip_prefix(name));
    let (Some(clip), Some(data)) = (flag("--probe-clip="), flag("--data=")) else {
        eprintln!("usage: wf-probe --probe-clip=<16 kHz mono WAV> --data=<folder>");
        return ExitCode::FAILURE;
    };
    match run(clip, data) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
