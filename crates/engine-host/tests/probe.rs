//! `wf-probe` as a developer runs it. The test that dictates is skipped by default:
//!
//! `WF_MODELS=…/wispr-free-notes/models cargo test -p wf-engine-host --release --test probe -- --ignored --nocapture`

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};

use tempfile::TempDir;
use wf_dictation::remote::engine_host_beside;

fn fixture() -> String {
    format!(
        "{}/../pipeline/tests/fixtures/speech_en.wav",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// A probe with its own home, so a path that isn't under `--data` lands where a test can see it.
struct Probe {
    child: Child,
    output: BufReader<ChildStdout>,
    home: TempDir,
}

impl Probe {
    fn start(data: &Path) -> Self {
        let home = tempfile::tempdir().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_wf-probe"))
            .arg(format!("--probe-clip={}", fixture()))
            .arg(format!("--data={}", data.display()))
            .env("HOME", home.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("binary runs");
        let output = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            output,
            home,
        }
    }

    fn say(&mut self, line: &str) {
        writeln!(self.child.stdin.as_mut().unwrap(), "{line}").unwrap();
    }

    fn line(&mut self) -> String {
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        line.trim_end().to_owned()
    }
}

#[test]
fn the_probe_needs_a_folder_of_its_own() {
    let output = Command::new(env!("CARGO_BIN_EXE_wf-probe"))
        .arg(format!("--probe-clip={}", fixture()))
        .stdin(Stdio::null())
        .output()
        .expect("binary runs");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn a_clip_it_cannot_read_is_an_error() {
    let data = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_wf-probe"))
        .arg("--probe-clip=/no/such/clip.wav")
        .arg(format!("--data={}", data.path().display()))
        .stdin(Stdio::null())
        .output()
        .expect("binary runs");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn the_probe_says_ready_and_leaves_the_home_folder_alone() {
    let data = tempfile::tempdir().unwrap();
    let mut probe = Probe::start(data.path());

    assert_eq!(probe.line(), "ready");
    probe.say("quit");

    assert!(probe.child.wait().unwrap().success());
    assert_eq!(fs::read_dir(probe.home.path()).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn a_binary_in_target_finds_the_engine_host_beside_it() {
    let probe = Path::new(env!("CARGO_BIN_EXE_wf-probe"));

    assert_eq!(
        engine_host_beside(probe),
        PathBuf::from(env!("CARGO_BIN_EXE_wf-engine-host"))
    );
}

#[cfg(target_os = "macos")]
mod real {
    use std::os::unix::fs::symlink;

    use wf_models::{Catalog, Tier};

    use super::*;

    /// `folder/<id>` for each model of the Lite tier, linked from where the test models live.
    fn link_lite_models(folder: &Path) {
        let source = PathBuf::from(std::env::var("WF_MODELS").expect("set WF_MODELS"));
        for model in Catalog::bundled().models_for_tier(Tier::Lite) {
            let linked = source.join(&model.id);
            if linked.is_dir() {
                symlink(linked, folder.join(&model.id)).unwrap();
            } else {
                let dir = folder.join(&model.id);
                fs::create_dir_all(&dir).unwrap();
                let file = &model.files[0].name;
                symlink(source.join(file), dir.join(file)).unwrap();
            }
        }
    }

    #[test]
    #[ignore = "needs models"]
    fn a_dictation_runs_through_a_real_host() {
        let data = tempfile::tempdir().unwrap();
        let models = data.path().join("models");
        fs::create_dir_all(&models).unwrap();
        link_lite_models(&models);
        fs::write(
            data.path().join("settings.toml"),
            "[pipeline]\ntier = \"lite\"\nllm_cleanup = false\n",
        )
        .unwrap();
        let mut probe = Probe::start(data.path());
        assert_eq!(probe.line(), "ready");

        probe.say("dictate");
        let line = probe.line();
        println!("{line}");

        assert!(line.starts_with("inserted: "));
        assert!(line.len() > "inserted: ".len(), "{line}");
        assert!(data.path().join("history.sqlite3").exists());
        probe.say("quit");
        assert!(probe.child.wait().unwrap().success());
    }
}
