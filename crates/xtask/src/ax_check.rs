//! `cargo xtask ax-check` reads the real dev bundle from a second process, the way an assistive
//! tool does, with today's labels as the contract. `cargo xtask bundle --dev` first.
//!
//! The app runs as `dev_app` says a tool may run it, and with `--behind`, so every window comes up
//! behind the others and nothing activates. The window opens through the status menu's own item.
//! `ax-walk` only reads and presses. What the result means is decided here.
//!
//! Settings is walked page by page. On each, a control is pressed the way an assistive tool would,
//! and this process then reads what reached the disk. Onboarding is walked step by step from
//! launch, on a reset data root. Nothing here brings up a question from macOS: a rehearsal's
//! permissions are made up, and a run that isn't a person's only logs what it would have asked.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail, ensure};
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
/// Where a run's folder picker lands, under its dev root.
const ELSEWHERE: &str = "elsewhere";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Node {
    pub role: String,
    pub identifier: String,
    pub title: String,
    pub enabled: bool,
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

    /// The first control with this identifier, in any window a menu opens.
    pub fn control(&self, identifier: &str) -> Option<&Node> {
        self.opened()
            .flat_map(|window| &window.nodes)
            .find(|node| node.identifier == identifier)
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

const SETTINGS_ITEM: &str = "Settings…";

pub const CONTRACTS: [Contract; 3] = [
    Contract {
        item: SETTINGS_ITEM,
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
        labels: &[
            "Step 1 of 4",
            "Allow Microphone access",
            "Open System Settings for Microphone access",
            "Continue",
        ],
    },
];

/// One step of a first run, in a rehearsal, whose permissions start out unasked and are answered
/// at once. Nothing here reaches a real prompt: the app logs what it would have asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepContract {
    pub title: &'static str,
    /// Controls that show on arrival.
    pub labels: &'static [&'static str],
    /// Pressed in order, each with what comes of it.
    pub presses: &'static [(&'static str, Outcome)],
}

pub const STEPS: [StepContract; 4] = [
    StepContract {
        title: "Step 1 of 4",
        labels: &[
            "Allow Microphone access",
            "Open System Settings for Microphone access",
            "Microphone access: Not allowed yet",
            "Built-in Microphone",
        ],
        presses: &[
            (
                "Open System Settings for Microphone access",
                Outcome::Said("would open x-apple.systempreferences"),
            ),
            (
                "Allow Microphone access",
                Outcome::Said("would ask for Microphone"),
            ),
        ],
    },
    StepContract {
        title: "Step 2 of 4",
        labels: &[
            "Back",
            "Allow Input Monitoring",
            "Allow Accessibility",
            "Input Monitoring: Not allowed yet",
            "Accessibility: Not allowed",
        ],
        presses: &[
            (
                "Allow Input Monitoring",
                Outcome::Said("would ask for InputMonitoring"),
            ),
            (
                "Allow Accessibility",
                Outcome::Said("would ask for Accessibility"),
            ),
        ],
    },
    StepContract {
        title: "Step 3 of 4",
        labels: &[
            "Back",
            "Quality tier",
            "Change models folder",
            "Open folder",
        ],
        presses: &[],
    },
    StepContract {
        title: "Step 4 of 4",
        labels: &[
            "Back",
            "Finish",
            "Try without the shortcut",
            "Shortcut: not tried",
            "Microphone: not tried",
            "Models: not tried",
        ],
        presses: &[
            ("Try without the shortcut", Outcome::Shows("Stop")),
            ("Stop", Outcome::Shows("Heard")),
        ],
    },
];

/// What the trial has shown once the mouse has run it.
const TRIED: [&str; 3] = ["Shortcut: failed", "Microphone: works", "Models: works"];

/// What happens when a control is pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// `settings.toml` comes to hold this line.
    Saved(&'static str),
    /// The app logs this.
    Said(&'static str),
    /// A control with this identifier shows up, in the window pressed.
    Shows(&'static str),
    /// A file in the dev root comes to hold this.
    Landed(&'static str),
}

/// One page of Settings, with its labels and what pressing some of them does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageContract {
    pub title: &'static str,
    pub labels: &'static [&'static str],
    pub presses: &'static [(&'static str, Outcome)],
}

pub const PAGES: [PageContract; 8] = [
    PageContract {
        title: "General",
        labels: &[
            "Change Push-to-talk",
            "Change Hands-free",
            "Change Cancel",
            "Change Paste last transcript",
            "Change Copy last transcript",
            "Change Start/stop meeting notes",
            "Choose automatically",
            "Test microphone",
            "Auto-detect",
            "Launch at login",
            "Show pill",
            "Show in Dock",
        ],
        presses: &[
            (
                "Choose automatically",
                Outcome::Shows("Move Built-in Microphone down"),
            ),
            ("Show pill", Outcome::Saved("show_pill = false")),
            ("Launch at login", Outcome::Shows("Notice")),
            ("Show in Dock", Outcome::Said("would show in Dock: true")),
        ],
    },
    PageContract {
        title: "Models",
        labels: &[
            "Open folder",
            "Change models folder",
            "Verify Parakeet TDT 0.6B v3 (25 European languages, int8)",
            "Delete Parakeet TDT 0.6B v3 (25 European languages, int8)",
            "Download Silero VAD v5",
        ],
        presses: &[
            // Moving would copy the rehearsal's 639 MB, so `tests/pages.rs` proves that on less.
            (
                "Change models folder",
                Outcome::Shows("Switch without moving"),
            ),
            ("Switch without moving", Outcome::Saved("models_folder = ")),
        ],
    },
    PageContract {
        title: "Pipeline",
        labels: &[
            "Quality tier",
            "Noise suppression",
            "Silence trimming (VAD)",
            "LLM cleanup",
            "Cleanup strength",
            "Stop hands-free after silence",
            "Silence before stopping",
            "Speech recognition",
            "Voice detection",
            "Cleanup model",
            "Keep models loaded",
        ],
        presses: &[("Noise suppression", Outcome::Saved("denoise = true"))],
    },
    PageContract {
        title: "Style & dictionary",
        labels: &[
            "Personal messages",
            "Work messages",
            "Email",
            "Other",
            "New word",
            "Add word",
            "Add corrected words automatically",
            "Say \"press enter\" to press Enter",
            "Add replacement",
            "Add snippet",
        ],
        presses: &[
            (
                "Add corrected words automatically",
                Outcome::Saved("auto_add_corrections = false"),
            ),
            ("Add replacement", Outcome::Shows("Wrong")),
            ("Add snippet", Outcome::Shows("Trigger phrase")),
        ],
    },
    PageContract {
        title: "Meetings",
        labels: &[
            "Suggest notes when a meeting starts",
            "Live transcript",
            "Keep the audio after the transcript is final",
            "Maximum length",
            "Refine pass",
            "Tell speakers apart",
            "Summary template",
        ],
        presses: &[("Live transcript", Outcome::Saved("live_transcript = false"))],
    },
    PageContract {
        title: "Sounds & media",
        labels: &["Sounds", "While dictating"],
        presses: &[("Sounds", Outcome::Saved("sounds = false"))],
    },
    PageContract {
        title: "Privacy & data",
        labels: &[
            "Context awareness",
            "History",
            "Open folder",
            "Export all",
            "Delete all data",
        ],
        presses: &[
            (
                "Context awareness",
                Outcome::Saved("context_awareness = false"),
            ),
            ("Export all", Outcome::Landed("exports")),
        ],
    },
    PageContract {
        title: "About",
        labels: &["Check for updates"],
        presses: &[(
            "Check for updates",
            Outcome::Said("would open https://github.com"),
        )],
    },
];

impl PageContract {
    fn contract(&self) -> Contract {
        Contract {
            item: SETTINGS_ITEM,
            window: SETTINGS,
            title: self.title,
            labels: self.labels,
        }
    }
}

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
        Self::start_with(walker, &["--rehearse", "--behind"], items, prefs)
    }

    fn start_with(
        walker: &Path,
        flags: &[&str],
        items: &[&str],
        prefs: Option<&str>,
    ) -> anyhow::Result<Self> {
        let home = Home::new("ax-check")?;
        if let Some(prefs) = prefs {
            fs::write(home.0.join(DEV_ROOT).join("pill.toml"), prefs)?;
        }
        let log = home.0.join("app.log");
        let elsewhere = home.0.join(DEV_ROOT).join(ELSEWHERE);
        fs::create_dir_all(&elsewhere)?;
        let mut args: Vec<_> = flags.iter().map(|flag| (*flag).to_owned()).collect();
        args.push(format!("--pick-folder={}", elsewhere.display()));
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
            if Instant::now() >= deadline {
                // "took the focus" in here tells a real activation from a report of one.
                bail!("after {WAIT:?}: {found:?}\nthe app said:\n{}", self.said()?);
            }
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

    fn root(&self) -> PathBuf {
        self.home.0.join(DEV_ROOT)
    }

    fn wait_for_settings(&self, line: &str) -> anyhow::Result<()> {
        let settings = self.root().join("settings.toml");
        let deadline = Instant::now() + WAIT;
        while !fs::read_to_string(&settings)?.contains(line) {
            ensure!(Instant::now() < deadline, "{line:?} never reached the disk");
            thread::sleep(POLL);
        }
        Ok(())
    }

    fn wait_for_file(&self, folder: &str) -> anyhow::Result<()> {
        let folder = self.root().join(folder);
        let deadline = Instant::now() + WAIT;
        while fs::read_dir(&folder).map_or(true, |mut files| files.next().is_none()) {
            ensure!(Instant::now() < deadline, "nothing landed in {folder:?}");
            thread::sleep(POLL);
        }
        Ok(())
    }

    fn wait_for_control(&self, window: &str, control: &str) -> anyhow::Result<()> {
        self.walk_until(|walk| {
            let open = walk.opened().find(|open| open.identifier == window);
            let found =
                open.is_some_and(|open| open.nodes.iter().any(|node| node.identifier == control));
            if found {
                Vec::new()
            } else {
                vec![format!("no {control}")]
            }
        })?;
        Ok(())
    }

    fn press(&self, window: &str, control: &str, outcome: Outcome) -> anyhow::Result<()> {
        self.walker_does("press", &[window, control])?;
        match outcome {
            Outcome::Saved(line) => self.wait_for_settings(line),
            Outcome::Said(line) => self.wait_to_say(line),
            Outcome::Shows(control) => self.wait_for_control(window, control),
            Outcome::Landed(folder) => self.wait_for_file(folder),
        }
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

/// Every page by its row: its title, its labels, and what its controls do to the disk.
fn check_the_pages(walker: &Path) -> anyhow::Result<()> {
    let prefs = "edge = \"bottom\"\nsounds = false\n";
    let run = Run::start(walker, &[SETTINGS_ITEM], Some(prefs))?;
    let app = run.pid();
    run.walk_until(|walk| broken(&PAGES[0].contract(), walk, app))?;
    for page in &PAGES {
        run.walker_does("select", &[SETTINGS, page.title])?;
        let contract = page.contract();
        let walk = run
            .walk_until(|walk| broken(&contract, walk, app))
            .with_context(|| page.title)?;
        for (control, outcome) in page.presses {
            run.press(SETTINGS, control, *outcome)
                .with_context(|| format!("{} {control}", page.title))?;
        }
        let nodes: usize = walk.opened().map(|window| window.nodes.len()).sum();
        println!(
            "{}: {} labels among {nodes} nodes, {} presses",
            page.title,
            page.labels.len(),
            page.presses.len()
        );
    }
    run.finish()
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

/// A reset data root's first run, by itself from launch to the trial, pressed the way an assistive
/// tool would. Continue does nothing until a step has what it needs.
fn check_the_first_run(walker: &Path) -> anyhow::Result<()> {
    let flags = ["--rehearse", "--behind", "--setup"];
    let run = Run::start_with(walker, &flags, &[], None)?;
    let app = run.pid();
    for (place, step) in STEPS.iter().enumerate() {
        let contract = Contract {
            item: "",
            window: SETUP,
            title: SETUP,
            labels: step.labels,
        };
        let walk = run
            .walk_until(|walk| {
                let mut problems = broken(&contract, walk, app);
                if walk.control(step.title).is_none() {
                    problems.push(format!("not on {}", step.title));
                }
                problems
            })
            .with_context(|| step.title)?;
        // Only these have anything to wait for: a rehearsal's models are there.
        if !step.presses.is_empty() && place + 1 < STEPS.len() {
            ensure!(!can_continue(&walk), "{} lets the user on", step.title);
        }
        for (control, outcome) in step.presses {
            run.press(SETUP, control, *outcome)
                .with_context(|| format!("{} {control}", step.title))?;
        }
        if place + 1 < STEPS.len() {
            let next = STEPS[place + 1].title;
            run.walk_until(|walk| {
                if can_continue(walk) {
                    Vec::new()
                } else {
                    vec!["Continue stays off".to_owned()]
                }
            })
            .with_context(|| step.title)?;
            run.press(SETUP, "Continue", Outcome::Shows(next))
                .with_context(|| step.title)?;
        }
        let nodes: usize = walk.opened().map(|window| window.nodes.len()).sum();
        println!(
            "{}: {} labels among {nodes} nodes, {} presses",
            step.title,
            step.labels.len(),
            step.presses.len()
        );
    }
    for label in TRIED {
        run.wait_for_control(SETUP, label)?;
    }
    run.walker_does("close", &[SETUP])?;
    run.wait_to_say(&format!("released {SETUP}"))?;
    let settings = fs::read_to_string(run.root().join("settings.toml"))?;
    ensure!(
        !settings.contains("finished = true"),
        "closing isn't finishing"
    );
    run.finish()?;
    println!("{SETUP}: from launch to the trial, nothing asked of macOS");
    Ok(())
}

fn can_continue(walk: &Walk) -> bool {
    walk.control("Continue").is_some_and(|node| node.enabled)
}

/// A run with the real `permissions::check` under it, on a reset data root. It opens on all four
/// steps, at the first the Mac leaves to do, and only ever reads: this run isn't a person's, so
/// "Allow…" would stay a line in its log.
fn check_a_real_look(walker: &Path) -> anyhow::Result<()> {
    let run = Run::start_with(walker, &["--behind", "--setup"], &[], None)?;
    let app = run.pid();
    let walk = run.walk_until(|walk| {
        let front = if walk.front_pid == app {
            vec!["the app is in front".to_owned()]
        } else {
            Vec::new()
        };
        let on_a_step = walk
            .opened()
            .flat_map(|window| &window.nodes)
            .any(|node| node.identifier.ends_with(" of 4"));
        if on_a_step {
            front
        } else {
            vec!["on no step of 4".to_owned()]
        }
    })?;
    let steps = walk.opened().flat_map(|window| &window.nodes);
    let step = steps
        .map(|node| node.identifier.as_str())
        .find(|identifier| identifier.ends_with(" of 4"));
    run.finish()?;
    println!(
        "{SETUP}: opens at launch on what the Mac really says, at {}",
        step.unwrap_or_default()
    );
    Ok(())
}

/// The models step shares its files with Settings, so a walk with that step closes it.
fn check_settings_closes(walker: &Path) -> anyhow::Result<()> {
    let [settings, _, setup] = CONTRACTS;
    let run = Run::start(walker, &[settings.item, setup.item], None)?;
    let app = run.pid();
    run.walk_until(|walk| broken(&setup, walk, app))?;
    run.wait_to_say(&format!("released {SETTINGS}"))?;
    run.finish()?;
    println!("{SETTINGS}: closes for the models step");
    Ok(())
}

pub fn run() -> anyhow::Result<()> {
    let walker = build_walker()?;
    for contract in &CONTRACTS {
        check_a_window(&walker, contract).with_context(|| contract.window)?;
    }
    check_the_pages(&walker)?;
    check_the_rule(&walker)?;
    check_the_first_run(&walker)?;
    check_a_real_look(&walker)?;
    check_settings_closes(&walker)
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
