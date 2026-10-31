//! Checks that run the built dev bundle, the only way a tool may run it: its binary directly, with
//! `HOME` on a temp folder and no `--by-hand`, so it can't reach the user's data, take the focus,
//! listen to the keys or catch a click. `cargo xtask bundle --dev` first.
//!
//! `cargo xtask shots --out dir` takes every pill state over its own backdrop, once per edge the
//! pill docks to, and every window in light and dark. Read `sheet.png` in each edge's folder and
//! the pictures in `windows`. `cargo xtask footprint` reports what the idle app costs, against the
//! spec's goal, and what it holds once Settings has been used and closed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail, ensure};

use crate::bundle::{DEV, target_dir, tool};

const EDGES: [&str; 3] = ["bottom", "left", "right"];
/// Every edge's pictures take under a minute.
const SHOTS_LIMIT: Duration = Duration::from_secs(120);
pub(crate) const QUIT_LIMIT: Duration = Duration::from_secs(10);
/// Long enough for the footprint to settle.
const IDLE: Duration = Duration::from_secs(30);
/// The spec's goal for an idle app before any window opens.
const IDLE_GOAL_MB: f64 = 25.0;
/// The spec's goal for an idle app after every window has been used and closed.
const USED_GOAL_MB: f64 = 55.0;
/// The spec's wait after closing a window, before it measured.
const SETTLE: Duration = Duration::from_secs(20);
/// Rounds of every Settings page: one, and then the spec's reopening five times more.
const TOURS: [u32; 2] = [1, 6];
const TOUR_LIMIT: Duration = Duration::from_secs(120);
pub(crate) const POLL: Duration = Duration::from_millis(100);
/// What the app logs if anything ever activates it.
const TOOK_THE_FOCUS: &str = "took the focus";
pub(crate) const DEV_ROOT: &str = "Library/Application Support/wispr-free-dev";

/// What `vmmap --summary` calls the footprint, in MB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Footprint {
    pub now_mb: f64,
    pub peak_mb: f64,
}

pub fn footprint_in(summary: &str) -> Option<Footprint> {
    let value = |label: &str| {
        summary
            .lines()
            .find_map(|line| line.trim().strip_prefix(label))
            .and_then(megabytes)
    };
    Some(Footprint {
        now_mb: value("Physical footprint:")?,
        peak_mb: value("Physical footprint (peak):")?,
    })
}

/// `10.3M`, as `vmmap` prints a size.
fn megabytes(text: &str) -> Option<f64> {
    let text = text.trim();
    let split = text.len().checked_sub(1)?;
    let (number, unit) = (text.get(..split)?, text.get(split..)?);
    let number: f64 = number.parse().ok()?;
    match unit {
        "K" => Some(number / 1024.0),
        "M" => Some(number),
        "G" => Some(number * 1024.0),
        _ => None,
    }
}

/// How the footprint stands against the goal.
pub fn verdict(footprint: Footprint) -> String {
    verdict_against("idle footprint", footprint, IDLE_GOAL_MB)
}

fn verdict_against(what: &str, footprint: Footprint, goal: f64) -> String {
    let Footprint { now_mb, peak_mb } = footprint;
    let gap = if now_mb < goal {
        format!("under the {goal} MB goal")
    } else {
        format!("{:.1} MB over the {goal} MB goal", now_mb - goal)
    };
    format!("{what} {now_mb:.1} MB, peak {peak_mb:.1} MB: {gap}")
}

/// The pill file that docks the app to `edge`, as `Prefs` reads it.
pub fn pill_file(edge: &str) -> String {
    format!("edge = \"{edge}\"\nsounds = false\n")
}

/// A temp `HOME`, gone when this is.
pub(crate) struct Home(pub(crate) PathBuf);

impl Home {
    pub(crate) fn new(name: &str) -> anyhow::Result<Self> {
        let home = std::env::temp_dir().join(format!("wf-{name}-{}", std::process::id()));
        if home.exists() {
            fs::remove_dir_all(&home)?;
        }
        fs::create_dir_all(home.join(DEV_ROOT))?;
        Ok(Self(home))
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn binary() -> anyhow::Result<PathBuf> {
    let binary = target_dir().join(DEV.app).join("Contents/MacOS/wispr-free");
    ensure!(
        binary.exists(),
        "no dev bundle: run `cargo xtask bundle --dev`"
    );
    Ok(binary)
}

fn start(home: &Path, args: &[String]) -> anyhow::Result<Child> {
    Command::new(binary()?)
        .args(args)
        .env("HOME", home)
        .current_dir("/")
        .stderr(Stdio::piped())
        .spawn()
        .context("starting the dev bundle")
}

/// Starts it with what it says going to `log`, where a check can read it while it runs.
pub(crate) fn start_logging(home: &Path, args: &[String], log: &Path) -> anyhow::Result<Child> {
    Command::new(binary()?)
        .args(args)
        .env("HOME", home)
        .current_dir("/")
        .stderr(fs::File::create(log)?)
        .spawn()
        .context("starting the dev bundle")
}

/// Holds a run to what it logged: the app says so if anything ever activates it.
pub fn no_focus_taken(said: &str) -> anyhow::Result<()> {
    ensure!(!said.contains(TOOK_THE_FOCUS), "it took the focus: {said}");
    Ok(())
}

/// Waits for `app` to end by itself. One that hangs is killed, and that's an error.
pub(crate) fn wait_for_exit(app: &mut Child, limit: Duration) -> anyhow::Result<String> {
    let deadline = Instant::now() + limit;
    while app.try_wait()?.is_none() {
        if Instant::now() >= deadline {
            app.kill()?;
            app.wait()?;
            bail!("the app didn't finish in {limit:?}");
        }
        thread::sleep(POLL);
    }
    let mut said = String::new();
    if let Some(stderr) = app.stderr.as_mut() {
        std::io::Read::read_to_string(stderr, &mut said)?;
    }
    Ok(said)
}

pub fn shots(mut args: impl Iterator<Item = String>) -> anyhow::Result<()> {
    let mut out = target_dir().join("shots");
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => out = args.next().context("--out needs a folder")?.into(),
            other => bail!("unknown flag {other:?}"),
        }
    }
    edge_shots(&out)?;
    window_shots(&out.join("windows"))
}

fn edge_shots(out: &Path) -> anyhow::Result<()> {
    for edge in EDGES {
        let home = Home::new("shots")?;
        fs::write(home.0.join(DEV_ROOT).join("pill.toml"), pill_file(edge))?;
        let folder = out.join(edge);
        let mut app = start(
            &home.0,
            &[
                "--rehearse".to_owned(),
                format!("--shots={}", folder.display()),
            ],
        )?;
        let said = wait_for_exit(&mut app, SHOTS_LIMIT)?;
        ensure!(said.contains("shots taken"), "{edge}: {said}");
        no_focus_taken(&said)?;
        println!("{}", folder.join("sheet.png").display());
    }
    Ok(())
}

fn window_shots(folder: &Path) -> anyhow::Result<()> {
    let home = Home::new("shots")?;
    let mut app = start(
        &home.0,
        &[
            "--rehearse".to_owned(),
            "--behind".to_owned(),
            format!("--window-shots={}", folder.display()),
        ],
    )?;
    let said = wait_for_exit(&mut app, SHOTS_LIMIT)?;
    ensure!(said.contains("shots taken"), "windows: {said}");
    no_focus_taken(&said)?;
    println!("{}", folder.display());
    Ok(())
}

pub fn footprint() -> anyhow::Result<()> {
    let home = Home::new("footprint")?;
    let mut app = start(&home.0, &[])?;
    thread::sleep(IDLE);
    let summary = tool("vmmap")
        .args(["--summary", &app.id().to_string()])
        .text();
    let stopped = tool("kill").arg(app.id().to_string()).succeed();
    let said = stopped.and_then(|()| wait_for_exit(&mut app, QUIT_LIMIT));
    let summary = summary?;
    ensure!(
        said?.contains("shell closed"),
        "the app didn't quit cleanly"
    );
    let footprint = footprint_in(&summary).context("vmmap gave no footprint")?;
    println!("{}", verdict(footprint));
    for rounds in TOURS {
        settings_footprint(rounds)?;
    }
    Ok(())
}

/// Walks every Settings page `rounds` times, closes it and lets it settle, as the spec measured.
fn settings_footprint(rounds: u32) -> anyhow::Result<()> {
    let home = Home::new("footprint")?;
    let log = home.0.join("app.log");
    let args = ["--behind".to_owned(), format!("--tour={rounds}")];
    let mut app = start_logging(&home.0, &args, &log)?;
    let deadline = Instant::now() + TOUR_LIMIT;
    while !fs::read_to_string(&log)?.contains("toured") {
        ensure!(Instant::now() < deadline, "the tour didn't finish");
        thread::sleep(POLL);
    }
    thread::sleep(SETTLE);
    let summary = tool("vmmap")
        .args(["--summary", &app.id().to_string()])
        .text();
    tool("kill").arg(app.id().to_string()).succeed()?;
    wait_for_exit(&mut app, QUIT_LIMIT)?;
    let said = fs::read_to_string(&log)?;
    ensure!(said.contains("shell closed"), "it didn't quit: {said}");
    no_focus_taken(&said)?;
    let footprint = footprint_in(&summary?).context("vmmap gave no footprint")?;
    let what = format!("after {rounds} round(s) of Settings, closed for {SETTLE:?}:");
    println!("{}", verdict_against(&what, footprint, USED_GOAL_MB));
    Ok(())
}
