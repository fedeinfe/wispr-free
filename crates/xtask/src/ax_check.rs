//! `cargo xtask ax-check` reads the real dev bundle from a second process, the way an assistive
//! tool does, with today's labels as the contract. `cargo xtask bundle --dev` first.
//!
//! The app runs as `dev_app` says a tool may run it, and with `--behind`, so every window comes up
//! behind the others and nothing activates. The window opens through the status menu's own item.
//! `ax-walk` only reads and presses. What the result means is decided here.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, ensure};
use serde::Deserialize;

use crate::bridge::apple_dir;
use crate::bundle::{SWIFT_JOBS, tool};
use crate::dev_app::{
    DEV_ROOT, Home, POLL, QUIT_LIMIT, no_focus_taken, start_logging, wait_for_exit,
};

const WALKER: &str = "ax-walk";
const WAIT: Duration = Duration::from_secs(15);
const LAST_PAGE: &str = "Privacy & data";
const LAST_PAGE_NAME: &str = "privacy";
const NEXT_PAGE: &str = "General";
const NEXT_PAGE_NAME: &str = "general";
const SETTINGS: &str = "Settings";
const SETUP: &str = "Set up Wispr Free";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Node {
    pub role: String,
    pub identifier: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Window {
    pub identifier: String,
    pub title: String,
    pub nodes: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Walk {
    pub trusted: bool,
    pub front_pid: i32,
    pub windows: Vec<Window>,
}

impl Walk {
    /// The windows a menu opens. The pill's panel is there too, with no identifier.
    pub fn opened(&self) -> impl Iterator<Item = &Window> {
        self.windows
            .iter()
            .filter(|window| !window.identifier.is_empty())
    }
}

/// What one window has to show. Its labels are the Slint app's, unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Contract {
    /// The status menu item that opens it.
    pub item: &'static str,
    pub window: &'static str,
    pub title: &'static str,
    pub labels: &'static [&'static str],
}

pub const CONTRACTS: [Contract; 3] = [
    Contract {
        item: "Settings…",
        window: SETTINGS,
        title: LAST_PAGE,
        labels: &[
            "General",
            "Models",
            "Pipeline",
            "Style & dictionary",
            "Meetings",
            "Sounds & media",
            "Privacy & data",
            "About",
        ],
    },
    Contract {
        item: "Meetings…",
        window: "Meetings",
        title: "Meetings",
        labels: &["Summary", "Notes", "Transcript", "Chat"],
    },
    Contract {
        item: "Finish setup…",
        window: SETUP,
        title: SETUP,
        labels: &["Back", "Continue"],
    },
];

/// What `walk` got wrong about `contract`, in words. Empty when it holds.
pub fn broken(contract: &Contract, walk: &Walk, app: i32) -> Vec<String> {
    let mut broken = Vec::new();
    if walk.front_pid == app {
        broken.push("the app is in front".to_owned());
    }
    let Some(window) = walk
        .windows
        .iter()
        .find(|window| window.identifier == contract.window)
    else {
        broken.push(format!("no window {}", contract.window));
        return broken;
    };
    if window.title != contract.title {
        broken.push(format!(
            "{} is titled {:?}, not {:?}",
            contract.window, window.title, contract.title
        ));
    }
    for label in contract.labels {
        if !window.nodes.iter().any(|node| node.identifier == *label) {
            broken.push(format!("{} has no {label}", contract.window));
        }
    }
    broken
}

/// One start of the app, gone when this is.
struct Run {
    home: Home,
    app: Child,
    log: PathBuf,
    walker: PathBuf,
}

impl Run {
    fn start(walker: &Path, items: &[&str], prefs: Option<&str>) -> anyhow::Result<Self> {
        let home = Home::new("ax-check")?;
        if let Some(prefs) = prefs {
            fs::write(home.0.join(DEV_ROOT).join("pill.toml"), prefs)?;
        }
        let log = home.0.join("app.log");
        let mut args = vec!["--rehearse".to_owned(), "--behind".to_owned()];
        args.extend(items.iter().map(|item| format!("--pick={item}")));
        let app = start_logging(&home.0, &args, &log)?;
        Ok(Self {
            home,
            app,
            log,
            walker: walker.to_owned(),
        })
    }

    fn pid(&self) -> i32 {
        self.app.id() as i32
    }

    fn said(&self) -> anyhow::Result<String> {
        Ok(fs::read_to_string(&self.log)?)
    }

    fn walk(&self) -> anyhow::Result<Walk> {
        let printed = tool(&self.walker)
            .args(["walk", &self.pid().to_string()])
            .text()?;
        let walk: Walk = serde_json::from_str(&printed).context("what ax-walk printed")?;
        ensure!(
            walk.trusted,
            "ax-check needs Accessibility for this terminal"
        );
        Ok(walk)
    }

    /// Walks until `problems` finds none, for as long as a slow start can take.
    fn walk_until(&self, problems: impl Fn(&Walk) -> Vec<String>) -> anyhow::Result<Walk> {
        let deadline = Instant::now() + WAIT;
        loop {
            let walk = self.walk()?;
            let found = problems(&walk);
            if found.is_empty() {
                return Ok(walk);
            }
            ensure!(Instant::now() < deadline, "after {WAIT:?}: {found:?}");
            thread::sleep(POLL);
        }
    }

    fn wait_to_say(&self, line: &str) -> anyhow::Result<()> {
        let deadline = Instant::now() + WAIT;
        while !self.said()?.contains(line) {
            ensure!(Instant::now() < deadline, "the app never said {line:?}");
            thread::sleep(POLL);
        }
        Ok(())
    }

    fn walker_does(&self, verb: &str, args: &[&str]) -> anyhow::Result<()> {
        tool(&self.walker)
            .arg(verb)
            .arg(self.pid().to_string())
            .args(args.iter().copied())
            .succeed()
    }

    /// Asks it to quit, then holds it to no window having taken the focus.
    fn finish(mut self) -> anyhow::Result<()> {
        tool("kill").arg(self.app.id().to_string()).succeed()?;
        wait_for_exit(&mut self.app, QUIT_LIMIT)?;
        let said = self.said()?;
        ensure!(said.contains("shell closed"), "it didn't quit: {said}");
        no_focus_taken(&said)
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        if matches!(self.app.try_wait(), Ok(None)) {
            let _ = self.app.kill();
            let _ = self.app.wait();
        }
    }
}

fn check_a_window(walker: &Path, contract: &Contract) -> anyhow::Result<()> {
    let prefs =
        format!("edge = \"bottom\"\nsounds = false\nsettings_page = \"{LAST_PAGE_NAME}\"\n");
    let run = Run::start(walker, &[contract.item], Some(&prefs))?;
    let app = run.pid();
    let walk = run.walk_until(|walk| broken(contract, walk, app))?;
    if contract.window == SETTINGS {
        follow_a_page(&run)?;
    }
    run.walker_does("close", &[contract.window])?;
    run.wait_to_say(&format!("released {}", contract.window))?;
    run.walk_until(|walk| {
        walk.opened()
            .map(|window| window.identifier.clone())
            .collect()
    })?;
    run.finish()?;
    let nodes: usize = walk.opened().map(|window| window.nodes.len()).sum();
    println!(
        "{}: {} labels among {nodes} nodes, released on close",
        contract.window,
        contract.labels.len()
    );
    Ok(())
}

/// The title follows the page, and the page is remembered.
fn follow_a_page(run: &Run) -> anyhow::Result<()> {
    run.walker_does("select", &[SETTINGS, NEXT_PAGE])?;
    run.walk_until(|walk| {
        let titles: Vec<_> = walk.opened().map(|window| window.title.clone()).collect();
        if titles == [NEXT_PAGE] {
            Vec::new()
        } else {
            titles
        }
    })?;
    let prefs = run.home.0.join(DEV_ROOT).join("pill.toml");
    let kept = format!("settings_page = \"{NEXT_PAGE_NAME}\"");
    let deadline = Instant::now() + WAIT;
    while !fs::read_to_string(&prefs)?.contains(&kept) {
        ensure!(Instant::now() < deadline, "the page wasn't kept");
        thread::sleep(POLL);
    }
    Ok(())
}

/// Opening Settings during onboarding raises onboarding, on the real app. Picks run in order, so
/// once Meetings is up, Settings has had its chance.
fn check_the_rule(walker: &Path) -> anyhow::Result<()> {
    let [settings, meetings, setup] = CONTRACTS;
    let run = Run::start(walker, &[setup.item, settings.item, meetings.item], None)?;
    let walk = run.walk_until(|walk| broken(&meetings, walk, run.pid()))?;
    let open = |window: &str| walk.opened().any(|open| open.identifier == window);
    ensure!(open(setup.window), "onboarding didn't stay open");
    ensure!(!open(settings.window), "Settings opened during onboarding");
    run.finish()?;
    println!("{}: comes forward in Settings' place", setup.window);
    Ok(())
}

pub fn run() -> anyhow::Result<()> {
    let walker = build_walker()?;
    for contract in &CONTRACTS {
        check_a_window(&walker, contract).with_context(|| contract.window)?;
    }
    check_the_rule(&walker)
}

/// `cargo xtask bundle --dev` built it too, so this is usually nothing.
fn build_walker() -> anyhow::Result<PathBuf> {
    let package = apple_dir();
    let release = ["--configuration", "release", "--package-path"];
    tool("swift")
        .arg("build")
        .args(release)
        .arg(&package)
        .args(["--product", WALKER, "--jobs", SWIFT_JOBS])
        .succeed()?;
    let products = tool("swift")
        .arg("build")
        .args(release)
        .arg(&package)
        .arg("--show-bin-path")
        .text()?;
    Ok(PathBuf::from(products.trim()).join(WALKER))
}
