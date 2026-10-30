//! Headless checks of the Meetings window, safe on a machine in use: Slint's software renderer
//! draws into memory, so there is no OS window and no focus change, and all files live in a
//! temporary folder. Nothing records and no engine runs.
//!
//!   cargo run --release -p wispr-free --example meetings_probe -- render <folder>
//!   cargo run --release -p wispr-free --example meetings_probe -- memory [cycles]
//!
//! `memory` opens and closes the window through `MeetingsHost`, on a meeting of a thousand lines,
//! drawing every tab each time, and prints `phys_footprint` in MB (macOS). As in `settings_probe`,
//! the running app's GL memory and the OS window are not in these numbers.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use i_slint_backend_testing::{TestingBackend, TestingBackendOptions};
use slint::{ComponentHandle, LogicalSize, Model, Timer, TimerMode};
use wf_dictation::{ChatRun, ChatUpdate, MeetingNews, MeetingsStatus, SummaryMode, WorkStep};
use wf_meetings::{
    ActionItem, Channel, ChatTurn, Item, LiveLog, Meeting, MeetingStore, Segment, SpeakerId, Stage,
    StageState, Summary, transcript_hash,
};
use wispr_free::meetings_window::{
    MeetingActions, MeetingsBoard, MeetingsController, MeetingsHost, MeetingsParts,
};
use wispr_free::{MeetingsUi, MeetingsWindow, Prefs};

const TABS: i32 = 4;

#[derive(Default)]
struct Quiet {
    status: RefCell<MeetingsStatus>,
}

impl MeetingActions for Quiet {
    fn toggle(&self) {}
    fn resume(&self, _id: &str) {}
    fn finish(&self, _id: &str) {}
    fn summarize(&self, _id: &str, _mode: SummaryMode) {}
    fn cancel(&self, _id: &str) {}

    fn ask(&self, _id: &str, _question: &str) -> ChatRun {
        ChatRun::new(1, Arc::<AtomicBool>::default())
    }

    fn status(&self) -> MeetingsStatus {
        self.status.borrow().clone()
    }
}

const LINES: [(u64, Channel, Option<u32>, &str); 8] = [
    (
        4_000,
        Channel::You,
        None,
        "Thanks for joining. I want to settle the launch date today.",
    ),
    (
        11_000,
        Channel::Others,
        Some(1),
        "Marketing needs two weeks once the date is fixed.",
    ),
    (
        19_000,
        Channel::Others,
        Some(2),
        "Support can't take a launch before the new rota starts.",
    ),
    (
        31_000,
        Channel::You,
        None,
        "Then let's say the fourteenth, and I'll tell the board.",
    ),
    (
        42_000,
        Channel::Others,
        Some(1),
        "Fine. I'll send the press kit to legal by Friday.",
    ),
    (
        55_000,
        Channel::Others,
        Some(2),
        "I'll write the rota up and share it on Monday.",
    ),
    (
        68_000,
        Channel::You,
        None,
        "Good. One more thing: the price stays as it is.",
    ),
    (
        77_000,
        Channel::Others,
        Some(1),
        "Agreed, no change on price.",
    ),
];

fn seeded(store: &MeetingStore, title: &str, extra_lines: u64) -> Meeting {
    let mut meeting = store.create().expect("a meeting");
    meeting.begin_part().expect("a part");
    meeting
        .end_part(Duration::from_secs(95 + extra_lines * 3))
        .expect("a length");
    let mut log = LiveLog::open(&meeting).expect("the live log");
    for (start_ms, channel, speaker, text) in LINES {
        log.segment(&Segment {
            channel,
            speaker: speaker.map(SpeakerId),
            start_ms,
            end_ms: start_ms + 5_000,
            text: text.to_owned(),
        })
        .expect("a line");
    }
    for line in 0..extra_lines {
        log.segment(&Segment {
            channel: Channel::Others,
            speaker: Some(SpeakerId(1)),
            start_ms: 95_000 + line * 3_000,
            end_ms: 97_000 + line * 3_000,
            text: format!("Line {line} goes on for a while, as people do in long meetings."),
        })
        .expect("a line");
    }
    drop(log);
    meeting
        .set_notes("Ask finance about the rota budget.")
        .expect("notes");
    let transcript = meeting.transcript().expect("a transcript");
    let item = |text: &str, at_ms| Item {
        text: text.to_owned(),
        at_ms: Some(at_ms),
    };
    let summary = Summary {
        title: Some(title.to_owned()),
        tldr: "The launch is on the fourteenth and the price stays as it is.".to_owned(),
        key_points: vec![
            item("Marketing needs two weeks once the date is fixed", 11_000),
            item("Support is waiting for the new rota", 19_000),
        ],
        decisions: vec![
            item("The launch is on the fourteenth", 31_000),
            item("No change on price", 77_000),
        ],
        action_items: vec![
            ActionItem {
                task: "Send the press kit to legal".to_owned(),
                owner: Some("Speaker 1".to_owned()),
                due: Some("Friday".to_owned()),
                at_ms: Some(42_000),
            },
            ActionItem {
                task: "Tell the board".to_owned(),
                owner: Some("You".to_owned()),
                due: None,
                at_ms: Some(31_000),
            },
        ],
        partial: false,
        transcript_hash: transcript_hash(&transcript),
    };
    meeting.save_summary(&summary).expect("a summary");
    meeting
        .add_chat_turn(ChatTurn {
            question: "What were the action items?".to_owned(),
            answer: "- Speaker 1 sends the press kit to legal by Friday. [00:42]\n\
                     - You tell the board. [00:31]\n\
                     - Speaker 2 shares the rota on Monday. [00:56]"
                .to_owned(),
            asked_ms: 0,
            stopped: false,
            partial_transcript: false,
        })
        .expect("a chat turn");
    meeting
        .set_stage(Stage::Diarization, StageState::Done { at_ms: 1 })
        .expect("a stage");
    meeting
        .set_stage(Stage::Refine, StageState::Done { at_ms: 1 })
        .expect("a stage");
    meeting
        .set_stage(Stage::Summary, StageState::Done { at_ms: 1 })
        .expect("a stage");
    std::thread::sleep(Duration::from_millis(3));
    meeting
}

fn save_png(window: &MeetingsWindow, path: &Path) {
    let snapshot = window.window().take_snapshot().expect("the window renders");
    image::save_buffer(
        path,
        snapshot.as_bytes(),
        snapshot.width(),
        snapshot.height(),
        image::ColorType::Rgba8,
    )
    .expect("png is written");
}

/// Lets the disk thread and the event loop work for a moment, drawing as a real window would.
fn pump(controller: &MeetingsController, done: impl Fn(&MeetingsUi) -> bool) {
    for _ in 0..500 {
        controller.drain_events();
        let window = controller.window().window();
        window.take_snapshot().expect("the window renders");
        slint::platform::update_timers_and_animations();
        if done(&controller.window().global::<MeetingsUi>()) {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("the window didn't get there");
}

fn render(target: &Path) {
    std::fs::create_dir_all(target).expect("output folder");
    let folder = tempfile::tempdir().expect("temporary folder");
    let store = MeetingStore::new(folder.path().join("meetings"));
    seeded(&store, "Weekly check-in", 0);
    let launch = seeded(&store, "Launch date and the rota", 0);
    let id = launch.record().id.clone();
    let long = seeded(&store, "A very long meeting", 600);
    let long = long.record().id.clone();
    for (name, theme) in [("light", 1), ("dark", 2)] {
        let quiet = Rc::new(Quiet::default());
        let controller = MeetingsController::new(MeetingsParts {
            store: store.clone(),
            actions: Rc::clone(&quiet) as Rc<dyn MeetingActions>,
            theme,
            export_dir: folder.path().to_owned(),
            bring_to_front: Rc::new(|| {}),
        })
        .expect("window");
        let window = controller.window();
        window.window().set_size(LogicalSize::new(980.0, 680.0));
        window.show().expect("window shows");
        let ui = window.global::<MeetingsUi>();
        assert_eq!(window.global::<Prefs>().get_values().theme, theme);
        pump(&controller, |ui| ui.get_meetings().row_count() == 3);
        save_png(window, &target.join(format!("{name}-nothing-open.png")));
        ui.invoke_select(id.as_str().into(), -1);
        pump(&controller, |ui| ui.get_transcript().row_count() > 0);
        for (tab, file) in ["summary", "notes", "transcript", "chat"]
            .iter()
            .enumerate()
        {
            ui.set_tab(tab as i32);
            save_png(window, &target.join(format!("{name}-{file}.png")));
        }

        ui.invoke_select(long.as_str().into(), 400);
        pump(&controller, |ui| ui.get_jump_row() == 400);
        slint::platform::update_timers_and_animations();
        save_png(window, &target.join(format!("{name}-jumped-on-open.png")));
        // From another tab, on a meeting whose transcript hasn't been on screen yet.
        ui.set_tab(0);
        ui.invoke_select(id.as_str().into(), -1);
        pump(&controller, |ui| ui.get_transcript().row_count() < 100);
        ui.invoke_select(long.as_str().into(), -1);
        pump(&controller, |ui| ui.get_transcript().row_count() > 100);
        ui.invoke_jump(200);
        slint::platform::update_timers_and_animations();
        save_png(
            window,
            &target.join(format!("{name}-jumped-from-a-tab.png")),
        );
        ui.set_tab(0);

        // A meeting at work, with a stage that failed and an answer coming in.
        let mut failing = store.open(&id).expect("the meeting");
        let reason = "the speaker model is damaged".to_owned();
        let failed = StageState::Failed { reason };
        failing
            .set_stage(Stage::Diarization, failed)
            .expect("a stage");
        let step = WorkStep::Summarizing { done: 3, total: 8 };
        quiet.status.borrow_mut().work = vec![(id.clone(), step)];
        controller.news(MeetingNews::Working {
            id: id.clone(),
            step,
        });
        controller.news(MeetingNews::WorkDone {
            id: "another".to_owned(),
            problem: None,
        });
        ui.invoke_select(id.as_str().into(), -1);
        pump(&controller, |ui| {
            ui.get_stages()
                .row_data(0)
                .is_some_and(|stage| stage.state == 3)
        });
        ui.set_tab(3);
        ui.invoke_ask("Who talks to legal?".into());
        controller.news(MeetingNews::Chat {
            run: 1,
            update: ChatUpdate::Token("Speaker 1 sends the press kit".to_owned()),
        });
        save_png(window, &target.join(format!("{name}-working.png")));
        let mut fixed = store.open(&id).expect("the meeting");
        fixed
            .set_stage(Stage::Diarization, StageState::Done { at_ms: 1 })
            .expect("a stage");
        window.hide().expect("window hides");
    }
}

fn draw_every_tab(window: &MeetingsWindow) {
    let ui = window.global::<MeetingsUi>();
    for tab in 0..TABS {
        ui.set_tab(tab);
        window.window().take_snapshot().expect("the tab renders");
    }
}

#[cfg(target_os = "macos")]
fn footprint_mb() -> f64 {
    #[repr(C)]
    #[derive(Default)]
    struct RusageInfoV0 {
        uuid: [u8; 16],
        user_time: u64,
        system_time: u64,
        pkg_idle_wkups: u64,
        interrupt_wkups: u64,
        pageins: u64,
        wired_size: u64,
        resident_size: u64,
        phys_footprint: u64,
        proc_start_abstime: u64,
        proc_exit_abstime: u64,
    }
    unsafe extern "C" {
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut RusageInfoV0) -> i32;
    }
    let mut info = RusageInfoV0::default();
    unsafe { proc_pid_rusage(std::process::id() as i32, 0, &mut info) };
    info.phys_footprint as f64 / (1024.0 * 1024.0)
}

#[cfg(not(target_os = "macos"))]
fn footprint_mb() -> f64 {
    0.0
}

fn measure(cycles: u32) {
    let folder = tempfile::tempdir().expect("temporary folder");
    let store = MeetingStore::new(folder.path().join("meetings"));
    let id = seeded(&store, "A long meeting", 1_000).record().id.clone();
    let export_dir = folder.path().to_owned();
    let board = MeetingsBoard::default();
    let host = MeetingsHost::new(
        Rc::new(move || MeetingsParts {
            store: store.clone(),
            actions: Rc::new(Quiet::default()) as Rc<dyn MeetingActions>,
            theme: 1,
            export_dir: export_dir.clone(),
            bring_to_front: Rc::new(|| {}),
        }),
        &board,
    );
    println!("start,{:.1}", footprint_mb());
    println!("cycle,open_mb,closed_before_collect_mb,closed_mb");

    let step = Cell::new(0u32);
    let (open, closing) = (Cell::new(0.0), Cell::new(0.0));
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(100), move || {
        let tick = step.get();
        step.set(tick + 1);
        // 2 s open, then 2 s closed: longer than the teardown delay before the collect.
        match tick % 40 {
            0 if tick / 40 >= cycles => {
                let _ = slint::quit_event_loop();
            }
            0 => host.open().expect("window opens"),
            1 => {
                let window = host.window().and_then(|window| window.upgrade());
                let window = window.expect("window is open");
                window
                    .global::<MeetingsUi>()
                    .invoke_select(id.as_str().into(), -1);
            }
            19 => {
                let window = host.window().and_then(|window| window.upgrade());
                let window = window.expect("window is open");
                draw_every_tab(&window);
                open.set(footprint_mb());
                host.close();
            }
            22 => closing.set(footprint_mb()),
            39 => println!(
                "{},{:.1},{:.1},{:.1}",
                tick / 40 + 1,
                open.get(),
                closing.get(),
                footprint_mb()
            ),
            _ => {}
        }
    });
    slint::run_event_loop_until_quit().expect("event loop runs");
}

fn main() -> ExitCode {
    let backend = TestingBackend::new(TestingBackendOptions {
        mock_time: false,
        threading: true,
        renderer_name: Some("software".into()),
    });
    slint::platform::set_platform(Box::new(backend)).expect("headless platform");

    let mut args = std::env::args().skip(1);
    match (args.next().as_deref(), args.next()) {
        (Some("render"), Some(folder)) => render(Path::new(&folder)),
        (Some("memory"), cycles) => measure(cycles.map_or(10, |n| n.parse().expect("a number"))),
        _ => {
            eprintln!("usage: meetings_probe render <folder> | memory [cycles]");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
