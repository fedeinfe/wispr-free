//! The pill as the bridge drives it: ticks in, frames out, and what its card, menus and drag do.
//! Nothing here makes a sound or opens a device.
#![cfg(target_os = "macos")]

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tempfile::TempDir;
use wf_audio::InputDevice;
use wf_core::dictation::PillInput;
use wf_core::pill::dock::{Display, Point, Rect};
use wf_core::pill::waveform::BARS;
use wf_core::pill::{PillState, Sound, WINDOW_SIZE};
use wf_core::settings::{PipelineTier, SettingsStore};
use wf_dictation::{Dictation, Event};
use wf_shell::bridge::{AppWindow, EventSink, PillParts, PillScene, Shell, ShellEvent};
use wf_shell::flow::{OnEvent, parts_at};
use wf_shell::menus::{MenuAction, MenuEntry};
use wf_shell::paths::Paths;
use wf_shell::pill::{
    CURSOR_POLL_MS, FRAME_POLL_MS, HIDDEN_POLL_MS, PillCore, PillFrame, pill_state_named,
};
use wf_shell::prefs::Prefs;
use wf_shell::rehearsal::HEARD;
use wf_shell::settings::page::SettingsPage;

const WAIT: Duration = Duration::from_secs(10);
const SCREEN: Rect = Rect {
    x: 0.0,
    y: 0.0,
    width: 1000.0,
    height: 800.0,
};

fn display() -> Display {
    Display {
        bounds: SCREEN,
        work_area: SCREEN,
        units_per_point: 1.0,
    }
}

fn at(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

struct Recorder(Mutex<Sender<ShellEvent>>);

impl EventSink for Recorder {
    fn on_event(&self, event: ShellEvent) {
        let _ = self.0.lock().unwrap().send(event);
    }
}

/// Runs a call of `Shell` that has nothing to wait for.
fn now<T>(future: impl Future<Output = T>) -> T {
    use std::task::{Context, Poll, Waker};
    match std::pin::pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("nothing here waits"),
    }
}

/// Where the middle of the capsule is on screen, after a frame put the window at its dock.
fn over_capsule(origin: Point, frame: &PillFrame) -> Point {
    Point {
        x: origin.x + frame.capsule.x + frame.capsule.width / 2.0,
        y: origin.y + frame.capsule.y + frame.capsule.height / 2.0,
    }
}

const AWAY: Point = Point { x: 1.0, y: 1.0 };

struct Rig {
    folder: TempDir,
    shell: Arc<Shell>,
    events: Receiver<ShellEvent>,
    /// Sends the shell what the flow would.
    flow: OnEvent,
    cues: Arc<Mutex<Vec<Sound>>>,
    origin: Point,
}

impl Rig {
    fn new() -> Self {
        Self::on(TempDir::new().unwrap())
    }

    fn on(folder: TempDir) -> Self {
        let paths = Paths::under(folder.path(), "/nowhere/wf-engine-host".into());
        let (sender, events) = mpsc::channel();
        let store = SettingsStore::open(&paths.settings);
        let cues = Arc::new(Mutex::new(Vec::new()));
        let played = Arc::clone(&cues);
        let pill = PillParts {
            prefs: paths.pill.clone(),
            play: Box::new(move |sound| played.lock().unwrap().push(sound)),
            microphones: Box::new(|| {
                vec![InputDevice {
                    id: "usb-1".to_owned(),
                    name: "Desk mic".to_owned(),
                }]
            }),
        };
        let sink = Arc::new(Recorder(Mutex::new(sender)));
        let mut flow = None;
        let shell = Shell::wired(store, sink, pill, |settings, on_event| {
            flow = Some(Arc::clone(&on_event));
            Dictation::start(parts_at(&paths, settings, on_event))
        });
        let mut rig = Self {
            folder,
            shell,
            events,
            flow: flow.unwrap(),
            cues,
            origin: AWAY,
        };
        rig.origin = rig
            .tick_with_screens(AWAY)
            .move_to
            .expect("the first tick docks the window");
        rig
    }

    fn tick_with_screens(&self, cursor: Point) -> PillFrame {
        self.shell
            .pill_tick(Some(cursor), Some(vec![display()]), false)
            .unwrap()
    }

    fn tick(&self, cursor: Point) -> PillFrame {
        self.shell.pill_tick(Some(cursor), None, false).unwrap()
    }

    /// Hovers the capsule, which turns Resting into Ready.
    fn hover(&self) -> PillFrame {
        let frame = self.tick(AWAY);
        let over = over_capsule(self.origin, &frame);
        self.tick(over);
        self.tick(over)
    }

    fn release(&self) {
        now(self.shell.pill_release()).unwrap();
    }

    fn pick(&self, action: MenuAction) {
        now(self.shell.menu_action(action)).unwrap();
    }

    fn drain(&self) -> Vec<ShellEvent> {
        self.events.try_iter().collect()
    }

    fn cues(&self) -> Vec<Sound> {
        self.cues.lock().unwrap().clone()
    }

    fn wait_for_menu_with(&self, name: &str) {
        let deadline = Instant::now() + WAIT;
        loop {
            let menu = self.shell.pill_menu().unwrap();
            let found = menu.iter().any(|entry| {
                matches!(entry, MenuEntry::Submenu { title, entries }
                    if title == name && !entries.is_empty())
            });
            if found {
                return;
            }
            assert!(Instant::now() < deadline, "no {name} in the menu");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

mod the_core {
    use super::*;

    fn core() -> PillCore {
        let mut core = PillCore::new(Prefs::default());
        core.tick(at(0), Some(AWAY), Some(vec![display()]), false);
        core
    }

    #[test]
    fn a_toast_shows_for_four_seconds_and_outranks_the_error_it_lays_over() {
        let mut core = core();
        core.set_state(PillState::Error {
            reason: "no mic".to_owned(),
        });
        core.show_toast("Copied".to_owned());

        let shown = core.tick(at(3), Some(AWAY), None, false);
        assert_eq!(shown.tooltip.as_deref(), Some("Copied"));

        let gone = core.tick(at(4), Some(AWAY), None, false);
        assert_eq!(gone.tooltip.as_deref(), Some("no mic"));
    }

    #[test]
    fn a_toast_shows_with_the_pill_turned_off() {
        let mut core = core();
        core.set_shown(false);
        core.show_toast("Copied".to_owned());

        let frame = core.tick(at(1), Some(AWAY), None, false);

        assert_eq!(frame.shown, PillState::Hidden);
        assert_eq!(frame.tooltip.as_deref(), Some("Copied"));
    }

    #[test]
    fn a_question_nobody_answers_goes_away_after_thirty_seconds() {
        let mut core = core();
        core.suggest_meeting("Zoom".to_owned());

        let up = core.tick(at(29), Some(AWAY), None, false);
        let card = up.card.expect("the card is up");
        assert_eq!(card.text, "Take notes for this Zoom meeting?");
        assert_eq!(card.accept, "Take notes");
        assert_eq!(card.decline, "Not now");
        assert_eq!(card.place.width, 236.0);

        let gone = core.tick(at(30), Some(AWAY), None, false);
        assert_eq!(gone.card, None);
        assert_eq!(core.question(), None);
    }

    #[test]
    fn the_meeting_card_goes_when_a_meeting_records_and_a_text_question_stays() {
        let mut core = core();
        core.offer_copy("hello".to_owned());
        core.hide_meeting_card();
        assert!(core.question().is_some());

        core.suggest_meeting("Zoom".to_owned());
        core.hide_meeting_card();
        assert_eq!(core.question(), None);
    }

    #[test]
    fn the_poll_follows_what_the_pill_is_doing() {
        let mut core = core();
        let idle = core.tick(at(1), Some(AWAY), None, false);
        assert_eq!(idle.next_tick_ms, CURSOR_POLL_MS);

        core.set_state(PillState::ListeningPtt);
        let listening = core.tick(at(2), Some(AWAY), None, false);
        assert_eq!(listening.next_tick_ms, FRAME_POLL_MS);
        assert_eq!(listening.bars.len(), BARS);
        assert_eq!(listening.bar_scale, 1.0);

        core.set_state(PillState::Resting);
        core.set_shown(false);
        let hidden = core.tick(at(3), Some(AWAY), None, false);
        assert_eq!(hidden.next_tick_ms, HIDDEN_POLL_MS);
        assert_eq!(hidden.bars, Vec::<f32>::new());

        core.offer_copy("hello".to_owned());
        let asking = core.tick(at(4), Some(AWAY), None, false);
        assert_eq!(asking.next_tick_ms, CURSOR_POLL_MS);
    }

    #[test]
    fn the_meeting_keeps_its_shorter_bars() {
        let mut core = core();
        core.set_state(PillState::Meeting);

        let frame = core.tick(at(1), Some(AWAY), None, false);

        assert_eq!(frame.bars.len(), BARS);
        assert_eq!(frame.bar_scale, 0.85);
    }

    #[test]
    fn the_screens_are_asked_for_once_a_second() {
        let mut core = core();

        assert!(!core.tick(at(0), Some(AWAY), None, false).wants_displays);
        assert!(core.tick(at(1), Some(AWAY), None, false).wants_displays);
        let told = core.tick(at(1), Some(AWAY), Some(vec![display()]), false);
        assert!(!told.wants_displays);
    }

    #[test]
    fn a_state_with_no_bars_drops_the_level_at_once() {
        let mut core = core();
        core.set_state(PillState::ListeningPtt);
        core.set_level(1.0);
        let loud = core.tick(at(1), Some(AWAY), None, false);
        core.set_state(PillState::Processing);
        core.set_state(PillState::ListeningPtt);

        let quiet = core.tick(at(1), Some(AWAY), None, false);

        let peak = |bars: &[f32]| bars.iter().copied().fold(0.0, f32::max);
        assert!(peak(&quiet.bars) < peak(&loud.bars) / 2.0);
    }

    #[test]
    fn sounds_follow_the_setting() {
        let mut core = core();
        assert_eq!(core.set_state(PillState::ListeningPtt), Some(Sound::Start));

        core.set_sounds(false);
        assert_eq!(core.set_state(PillState::Resting), None);
    }

    #[test]
    fn every_state_has_a_command_line_name() {
        for (name, state) in [
            ("hidden", PillState::Hidden),
            ("resting", PillState::Resting),
            ("ready", PillState::Ready),
            ("listening", PillState::ListeningPtt),
            ("locked", PillState::ListeningHandsFree),
            ("processing", PillState::Processing),
            ("meeting", PillState::Meeting),
        ] {
            assert_eq!(pill_state_named(name), Some(state));
        }
        assert!(matches!(
            pill_state_named("error"),
            Some(PillState::Error { .. })
        ));
        assert_eq!(pill_state_named("sideways"), None);
    }

    #[test]
    fn the_made_up_level_stays_in_range() {
        for i in 0..10_000 {
            let level = wf_shell::pill::made_up_level(i as f32 * 0.01);
            assert!((0.0..=1.0).contains(&level), "{level}");
        }
    }
}

#[test]
fn a_tick_docks_the_window_and_says_so_only_when_it_moves() {
    let rig = Rig::new();

    assert_eq!(
        rig.origin,
        Point {
            x: (SCREEN.width - WINDOW_SIZE.width) / 2.0,
            y: SCREEN.height - WINDOW_SIZE.height,
        }
    );
    assert_eq!(rig.tick(AWAY).move_to, None);
}

#[test]
fn the_window_catches_the_mouse_only_over_the_capsule() {
    let rig = Rig::new();

    let frame = rig.tick(AWAY);
    assert!(!frame.hit_test);

    let frame = rig.tick(over_capsule(rig.origin, &frame));
    assert!(frame.hit_test);
}

#[test]
fn the_menu_is_only_for_a_pill_showing_ready() {
    let rig = Rig::new();
    assert_eq!(rig.shell.pill_menu().unwrap(), []);

    rig.hover();
    assert!(!rig.shell.pill_menu().unwrap().is_empty());

    rig.shell
        .stage(PillScene::State {
            state: PillState::Processing,
        })
        .unwrap();
    rig.hover();
    assert_eq!(rig.shell.pill_menu().unwrap(), []);
}

#[test]
fn what_the_flow_reports_reaches_the_pill_without_passing_through_swift() {
    let rig = Rig::new();

    (rig.flow)(Event::Pill(PillState::ListeningHandsFree));
    (rig.flow)(Event::Level(0.9));
    (rig.flow)(Event::Toast("hello".to_owned()));
    (rig.flow)(Event::NoTextField("words".to_owned()));
    std::thread::sleep(Duration::from_millis(30));

    let frame = rig.tick(AWAY);
    assert_eq!(frame.shown, PillState::ListeningHandsFree);
    assert_eq!(frame.capsule.width, 100.0);
    assert!(frame.bars.iter().any(|&bar| bar > 0.13), "{:?}", frame.bars);
    assert_eq!(frame.tooltip.as_deref(), Some("hello"));
    let card = frame.card.expect("the card is up");
    assert_eq!(card.accept, "Copy");
    assert_eq!(rig.cues(), [Sound::Start]);
}

#[test]
fn a_recorded_meeting_takes_the_question_down_and_flips_the_status_menu() {
    let rig = Rig::new();
    (rig.flow)(Event::SuggestMeeting("Zoom".to_owned()));
    assert!(rig.tick(AWAY).card.is_some());

    (rig.flow)(Event::Meeting(true));

    assert_eq!(rig.tick(AWAY).card, None);
    let menu = rig.shell.status_menu(false).unwrap();
    assert!(menu.contains(&MenuEntry::Item {
        title: "Stop meeting notes".to_owned(),
        action: MenuAction::ToggleMeeting,
    }));
}

#[test]
fn turning_sounds_off_in_the_settings_silences_the_pill() {
    let rig = Rig::new();
    let mut latest = rig.shell.settings().unwrap();
    latest.settings.media.sounds = false;
    now(Arc::clone(&rig.shell).save(latest.settings, latest.revision)).unwrap();

    (rig.flow)(Event::Pill(PillState::ListeningPtt));

    assert_eq!(rig.cues(), []);
}

#[test]
fn turning_the_pill_off_in_the_settings_hides_it_and_an_error_still_shows() {
    let rig = Rig::new();
    let mut latest = rig.shell.settings().unwrap();
    latest.settings.general.show_pill = false;
    now(Arc::clone(&rig.shell).save(latest.settings, latest.revision)).unwrap();
    assert_eq!(rig.tick(AWAY).shown, PillState::Hidden);

    (rig.flow)(Event::Pill(PillState::Error {
        reason: "no mic".to_owned(),
    }));

    assert!(matches!(rig.tick(AWAY).shown, PillState::Error { .. }));
}

#[test]
fn a_learned_word_shows_as_a_toast_on_the_pill() {
    let rig = Rig::new();

    (rig.flow)(Event::WordLearned("Wispr".to_owned()));

    assert_eq!(
        rig.tick(AWAY).tooltip.as_deref(),
        Some("Added “Wispr” to the dictionary")
    );
}

#[test]
fn a_no_on_the_card_takes_it_down_and_tells_the_flow_nothing() {
    let rig = Rig::new();
    (rig.flow)(Event::NoTextField("words".to_owned()));
    rig.drain();

    rig.shell.pill_answer(false).unwrap();

    assert_eq!(rig.tick(AWAY).card, None);
    assert_eq!(rig.drain(), []);
}

#[test]
fn a_drag_to_another_edge_is_saved_where_the_paths_say_and_comes_back() {
    let rig = Rig::new();
    let frame = rig.hover();
    let start = over_capsule(rig.origin, &frame);
    rig.tick(start);

    rig.shell.pill_press().unwrap();
    rig.tick(start);
    let left = Point {
        x: 20.0,
        y: SCREEN.height / 2.0,
    };
    rig.tick(left);
    let dragged = rig.tick(left);
    assert!(dragged.hit_test, "a drag keeps the mouse");
    rig.tick(left);
    rig.release();

    let pill = rig.folder.path().join("pill.toml");
    assert_eq!(Prefs::load(&pill).edge.as_str(), "left");

    let Rig { folder, shell, .. } = rig;
    drop(shell);
    let back = Rig::on(folder);
    assert_eq!(back.origin.x, 0.0, "it docks left after a restart");
}

#[test]
fn esc_during_a_drag_puts_the_pill_back_and_saves_nothing() {
    let rig = Rig::new();
    let frame = rig.hover();
    let start = over_capsule(rig.origin, &frame);
    rig.tick(start);
    rig.shell.pill_press().unwrap();
    rig.tick(start);
    let left = Point {
        x: 20.0,
        y: SCREEN.height / 2.0,
    };
    rig.tick(left);

    let cancelled = rig.shell.pill_tick(Some(left), None, true).unwrap();

    assert_eq!(cancelled.move_to, Some(rig.origin));
    rig.release();
    assert!(!rig.folder.path().join("pill.toml").exists());
}

#[test]
fn the_pill_menu_lists_the_microphones_and_a_choice_tops_the_ranking() {
    let rig = Rig::new();
    rig.hover();
    rig.wait_for_menu_with("Microphone");

    rig.pick(MenuAction::Microphone {
        id: "usb-1".to_owned(),
    });

    let settings = rig.shell.settings().unwrap().settings;
    assert_eq!(settings.general.microphones, ["usb-1"]);
}

#[test]
fn a_language_replaces_my_languages_and_auto_detect_empties_them() {
    let rig = Rig::new();

    rig.pick(MenuAction::Language {
        code: Some("it".to_owned()),
    });
    assert_eq!(
        rig.shell.settings().unwrap().settings.general.languages,
        ["it"]
    );

    rig.pick(MenuAction::Language { code: None });
    assert_eq!(
        rig.shell.settings().unwrap().settings.general.languages,
        Vec::<String>::new()
    );
}

#[test]
fn pipeline_quality_goes_through_the_tier_rules() {
    let rig = Rig::new();
    let before = rig.shell.settings().unwrap();

    rig.pick(MenuAction::PipelineQuality {
        tier: PipelineTier::Lite,
    });

    let after = rig.shell.settings().unwrap();
    assert_eq!(after.revision, before.revision + 1);
    assert_eq!(after.settings.pipeline.tier, PipelineTier::Lite);
}

#[test]
fn hiding_for_an_hour_hides_whatever_state_it_was_in() {
    let rig = Rig::new();

    rig.pick(MenuAction::HideForAnHour);

    assert_eq!(rig.tick(AWAY).shown, PillState::Hidden);
}

#[test]
fn what_needs_a_window_or_the_app_comes_back_as_an_event() {
    let rig = Rig::new();
    rig.drain();

    rig.pick(MenuAction::OpenSettings);
    rig.pick(MenuAction::OpenMeetings);
    rig.pick(MenuAction::OpenSettings);
    rig.pick(MenuAction::Quit);

    assert_eq!(
        rig.drain(),
        [
            ShellEvent::Open {
                window: AppWindow::Settings
            },
            ShellEvent::Open {
                window: AppWindow::Meetings
            },
            ShellEvent::Raise {
                window: AppWindow::Settings
            },
            ShellEvent::QuitAsked,
        ]
    );
}

#[test]
fn settings_asked_for_during_onboarding_raises_onboarding_until_it_closes() {
    let rig = Rig::new();
    rig.drain();

    rig.pick(MenuAction::OpenOnboarding);
    rig.pick(MenuAction::OpenSettings);
    rig.shell.window_closed(AppWindow::Onboarding).unwrap();
    rig.pick(MenuAction::OpenSettings);

    assert_eq!(
        rig.drain(),
        [
            ShellEvent::Open {
                window: AppWindow::Onboarding
            },
            ShellEvent::Raise {
                window: AppWindow::Onboarding
            },
            ShellEvent::Open {
                window: AppWindow::Settings
            },
        ]
    );
}

#[test]
fn finish_setup_is_offered_until_onboarding_finished() {
    let rig = Rig::new();
    let setup = MenuEntry::Item {
        title: "Finish setup…".to_owned(),
        action: MenuAction::OpenOnboarding,
    };
    assert_eq!(rig.shell.status_menu(false).unwrap()[0], setup);

    let current = rig.shell.settings().unwrap();
    let mut settings = current.settings;
    settings.onboarding.finished = true;
    now(Arc::clone(&rig.shell).save(settings, current.revision)).unwrap();

    assert!(!rig.shell.status_menu(false).unwrap().contains(&setup));
}

#[test]
fn the_page_settings_was_on_survives_a_restart() {
    let rig = Rig::new();
    assert_eq!(rig.shell.settings_page().unwrap(), SettingsPage::General);

    now(rig.shell.set_settings_page(SettingsPage::Privacy)).unwrap();

    assert_eq!(rig.shell.settings_page().unwrap(), SettingsPage::Privacy);
    let restarted = Rig::on(rig.folder);
    assert_eq!(
        restarted.shell.settings_page().unwrap(),
        SettingsPage::Privacy
    );
}

#[test]
fn the_debug_menu_plays_a_sound_and_a_picked_state_waves_by_itself() {
    let rig = Rig::new();

    rig.pick(MenuAction::DebugSound { sound: Sound::Lock });
    rig.pick(MenuAction::DebugState {
        state: PillState::ListeningPtt,
    });
    rig.tick(AWAY);
    std::thread::sleep(Duration::from_millis(200));

    let frame = rig.tick(AWAY);
    assert_eq!(frame.shown, PillState::ListeningPtt);
    assert_eq!(rig.cues(), [Sound::Lock, Sound::Start]);
}

mod a_rehearsal {
    use super::*;

    struct Run {
        _folder: TempDir,
        shell: Arc<Shell>,
        events: Receiver<ShellEvent>,
    }

    fn run() -> Run {
        let folder = TempDir::new().unwrap();
        let paths = Paths::under(folder.path(), "/nowhere/wf-engine-host".into());
        let (sender, events) = mpsc::channel();
        let sink = Arc::new(Recorder(Mutex::new(sender)));
        let shell = now(Shell::rehearse(paths, sink)).unwrap();
        Run {
            _folder: folder,
            shell,
            events,
        }
    }

    fn wait_for<T>(events: &Receiver<ShellEvent>, pick: impl Fn(&ShellEvent) -> Option<T>) -> T {
        let deadline = Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let event = events.recv_timeout(left).expect("the event in time");
            if let Some(found) = pick(&event) {
                return found;
            }
        }
    }

    fn wait_pill(run: &Run, wanted: PillState) {
        wait_for(&run.events, |event| {
            (*event
                == ShellEvent::Pill {
                    state: wanted.clone(),
                })
            .then_some(())
        });
    }

    /// The whole way: a click on the capsule, the real flow on fake engines, and the words coming
    /// back with the pill at rest again.
    #[test]
    fn a_click_on_the_pill_dictates_end_to_end() {
        let run = run();
        let screens = Some(vec![display()]);
        let first = run.shell.pill_tick(Some(AWAY), screens, false).unwrap();
        let origin = first.move_to.unwrap();
        let over = over_capsule(origin, &first);
        run.shell.pill_tick(Some(over), None, false).unwrap();
        let ready = run.shell.pill_tick(Some(over), None, false).unwrap();
        assert_eq!(ready.shown, PillState::Ready);

        run.shell.pill_press().unwrap();
        now(run.shell.pill_release()).unwrap();

        wait_pill(&run, PillState::ListeningHandsFree);
        wait_for(&run.events, |event| {
            matches!(event, ShellEvent::Level { value } if *value > 0.0).then_some(())
        });
        let listening = run.shell.pill_tick(Some(over), None, false).unwrap();
        assert_eq!(listening.shown, PillState::ListeningHandsFree);
        assert_eq!(listening.bars.len(), BARS);

        run.shell.on_pill(PillInput::Stop).unwrap();
        wait_pill(&run, PillState::Processing);
        let (mut heard, mut at_rest) = (None, false);
        let deadline = Instant::now() + WAIT;
        while heard.is_none() || !at_rest {
            let left = deadline.saturating_duration_since(Instant::now());
            match run.events.recv_timeout(left).expect("the end of the run") {
                ShellEvent::Recent { transcripts } => heard = transcripts.first().cloned(),
                ShellEvent::Pill { state } => at_rest = state == PillState::Resting,
                _ => {}
            }
        }

        let heard = heard.unwrap();
        assert!(heard.starts_with(HEARD), "{heard}");
        let hovered = run.shell.pill_tick(Some(over), None, false).unwrap();
        assert_eq!(hovered.shown, PillState::Ready);
        assert_eq!(rehearsed_recent(&run), [heard]);
    }

    fn rehearsed_recent(run: &Run) -> Vec<String> {
        let menu = run.shell.pill_menu().unwrap();
        menu.iter()
            .find_map(|entry| match entry {
                MenuEntry::Submenu { title, entries } if title == "Recent transcripts" => {
                    Some(entries.clone())
                }
                _ => None,
            })
            .expect("the menu is open to a hovered pill")
            .into_iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item { title, .. } => Some(title),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_yes_on_the_card_reaches_the_flow() {
        let run = run();
        run.shell
            .stage(PillScene::NoTextField {
                text: "words".to_owned(),
            })
            .unwrap();

        run.shell.pill_answer(true).unwrap();

        let toast = wait_for(&run.events, |event| match event {
            ShellEvent::Toast { text } => Some(text.clone()),
            _ => None,
        });
        assert!(toast.starts_with("Copied"), "{toast}");
    }
}
