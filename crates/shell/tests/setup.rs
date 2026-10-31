//! Onboarding as Swift reaches it: `Shell`'s calls and events, on the pages' thread, with the real
//! wiring in between. A rehearsal's machine and flow have nothing real under them, and the other
//! runs here stand on a fake machine, so nothing touches TCC, the keys or a microphone.

#[path = "support/memory_cap.rs"]
mod memory_cap;

use std::any::Any;
use std::cell::RefCell;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use wf_core::pill::PillState;
use wf_core::settings::SettingsStore;
use wf_dictation::Dictation;
use wf_platform::keys::KeySignal;
use wf_platform::permissions::{Access, Permission, Permissions};
use wf_shell::bridge::{AppWindow, EventSink, PillParts, PillScene, Shell, ShellEvent};
use wf_shell::menus::{MenuAction, MenuEntry};
use wf_shell::onboarding::desk::MachineHooks;
use wf_shell::onboarding::steps::Step;
use wf_shell::onboarding::trial::{Check, Phase};
use wf_shell::onboarding::{Machine, SetupPage};
use wf_shell::paths::Paths;
use wf_shell::rehearsal;
use wf_shell::settings::{Desktop, Mailbox};

/// Far more than any of this takes, short of a stuck run. The idle look is 5 s.
const WAIT: Duration = Duration::from_secs(10);
const MEMORY_LIMIT_MB: u64 = 1024;
const SETUP: AppWindow = AppWindow::Onboarding;

const GRANTED: Permissions = Permissions {
    microphone: Access::Granted,
    accessibility: Access::Granted,
    input_monitoring: Access::Granted,
};

const NO_MICROPHONE: Permissions = Permissions {
    microphone: Access::Denied,
    ..GRANTED
};

struct Recorder(Mutex<Sender<ShellEvent>>);

impl EventSink for Recorder {
    fn on_event(&self, event: ShellEvent) {
        let _ = self.0.lock().unwrap().send(event);
    }
}

/// What a fake machine was told, and what it answers.
struct FakeState {
    permissions: Permissions,
    armed: bool,
}

struct FakeMachine(Arc<Mutex<FakeState>>, MachineHooks);

impl Machine for FakeMachine {
    fn permissions(&self) -> Permissions {
        self.0.lock().unwrap().permissions
    }

    fn ask(&self, _permission: Permission) {}

    fn keys_listening(&self) -> bool {
        true
    }

    fn refresh_keys(&self, _permissions: &Permissions, _may_prompt: bool) {}

    fn arm_trial(&self, armed: bool) {
        self.0.lock().unwrap().armed = armed;
    }

    fn start_trial(&self) {}

    fn stop_trial(&self) {}

    fn set_blocked(&self, _reason: Option<String>) {}

    fn set_setup_needed(&self, needed: bool) {
        (self.1.set_setup_needed)(needed);
    }

    fn close_settings(&self) {
        (self.1.close_settings)();
    }

    fn reopen_app(&self) {
        (self.1.quit)();
    }
}

struct QuietDesktop;

impl Desktop for QuietDesktop {
    fn bring_to_front(&self) {}

    fn start_probes(&self, mailbox: &Mailbox) {
        rehearsal::report_machine(mailbox);
    }

    fn set_launch_at_login(&self, _enabled: bool) -> io::Result<()> {
        Ok(())
    }

    fn set_dock_icon_visible(&self, _visible: bool) {}

    fn open(&self, _target: &str) -> io::Result<()> {
        Ok(())
    }

    fn capture_keys(
        &self,
        _on_signal: Box<dyn FnMut(KeySignal) + Send>,
    ) -> Result<Box<dyn Any>, String> {
        Err("not here".to_owned())
    }

    fn test_microphone(
        &self,
        _ranking: Vec<String>,
        _on_level: Box<dyn FnMut(f32) + Send>,
    ) -> Result<Box<dyn Any>, String> {
        Err("not here".to_owned())
    }

    fn pick_folder(&self, _start: &Path, _done: Box<dyn FnOnce(PathBuf)>) {}

    fn look_for_microphones(&self, mailbox: &Mailbox) {
        rehearsal::report_microphones(mailbox);
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

struct Rig {
    folder: TempDir,
    shell: Arc<Shell>,
    events: Receiver<ShellEvent>,
    /// What's been heard while waiting for something else.
    heard: RefCell<Vec<ShellEvent>>,
    machine: Option<Arc<Mutex<FakeState>>>,
}

impl Rig {
    /// A first run that has nothing real under it, on the real wiring.
    fn rehearsal() -> Self {
        memory_cap::install(MEMORY_LIMIT_MB);
        let folder = TempDir::new().unwrap();
        let paths = Paths::under(folder.path(), "/nowhere/wf-engine-host".into());
        let (sender, events) = mpsc::channel();
        let sink = Arc::new(Recorder(Mutex::new(sender)));
        let shell = now(Shell::rehearse(paths, sink)).unwrap();
        Self {
            folder,
            shell,
            events,
            heard: RefCell::default(),
            machine: None,
        }
    }

    /// A run on a fake machine, with a rehearsal's models in place.
    fn on_a_fake_machine(finished: bool, permissions: Permissions) -> Self {
        memory_cap::install(MEMORY_LIMIT_MB);
        let folder = TempDir::new().unwrap();
        let paths = Paths::under(folder.path(), "/nowhere/wf-engine-host".into());
        let (sender, events) = mpsc::channel();
        let sink = Arc::new(Recorder(Mutex::new(sender)));
        let store = SettingsStore::open(&paths.settings);
        store
            .update(|settings| {
                *settings = rehearsal::settings();
                settings.onboarding.finished = finished;
            })
            .unwrap();
        let pill = PillParts {
            prefs: paths.pill.clone(),
            play: Box::new(|_| {}),
            microphones: Box::new(Vec::new),
        };
        let machine = Arc::new(Mutex::new(FakeState {
            permissions,
            armed: false,
        }));
        let for_shell = Arc::clone(&machine);
        let shell = Shell::wired_with(
            store,
            sink,
            pill,
            paths.clone(),
            Some(Box::new(|| Rc::new(QuietDesktop))),
            Some(Box::new(move |_clock, hooks| {
                Rc::new(FakeMachine(for_shell, hooks))
            })),
            |settings, on_event| Dictation::start(rehearsal::parts(&paths, settings, on_event)),
        );
        Self {
            folder,
            shell,
            events,
            heard: RefCell::default(),
            machine: Some(machine),
        }
    }

    fn set_permissions(&self, permissions: Permissions) {
        let machine = self.machine.as_ref().expect("a fake machine");
        machine.lock().unwrap().permissions = permissions;
    }

    fn pick(&self, action: MenuAction) {
        now(self.shell.menu_action(action)).unwrap();
    }

    fn page(&self) -> Option<SetupPage> {
        now(self.shell.steps()).unwrap()
    }

    fn page_until(&self, what: &str, done: impl Fn(&SetupPage) -> bool) -> SetupPage {
        self.page_within(WAIT, what, done)
    }

    fn page_within(
        &self,
        limit: Duration,
        what: &str,
        done: impl Fn(&SetupPage) -> bool,
    ) -> SetupPage {
        let deadline = Instant::now() + limit;
        loop {
            if let Some(page) = self.page().filter(&done) {
                return page;
            }
            assert!(
                Instant::now() < deadline,
                "{what}, on {:?}",
                self.page().map(|page| (page.step, page.can_continue))
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Everything that has come out so far, and everything that came out before.
    fn seen(&self) -> Vec<ShellEvent> {
        self.heard.borrow_mut().extend(self.events.try_iter());
        self.heard.borrow().clone()
    }

    fn has_seen(&self, event: &ShellEvent) -> bool {
        self.seen().contains(event)
    }

    fn wait_for(&self, event: &ShellEvent) {
        let deadline = Instant::now() + WAIT;
        while !self.has_seen(event) {
            assert!(Instant::now() < deadline, "no {event:?}");
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Asks for `window` from the menu, and forgets what came before.
    fn forget_events(&self) {
        self.seen();
        self.heard.borrow_mut().clear();
    }

    fn windows_said(&self) -> Vec<ShellEvent> {
        let events = self.seen().into_iter();
        events
            .filter(|event| {
                matches!(
                    event,
                    ShellEvent::Open { .. }
                        | ShellEvent::OpenQuietly { .. }
                        | ShellEvent::Raise { .. }
                        | ShellEvent::Close { .. }
                )
            })
            .collect()
    }

    /// Swift's window has gone, for any reason.
    fn swift_closed(&self, window: AppWindow) {
        self.shell.window_closed(window).unwrap();
    }

    /// Makes sure the pages' thread has done what it was told.
    fn settle(&self) {
        self.page();
    }

    fn finished_on_disk(&self) -> bool {
        SettingsStore::open(self.folder.path().join("settings.toml"))
            .get()
            .onboarding
            .finished
    }

    fn offers_setup(&self) -> bool {
        let menu = self.shell.status_menu(false).unwrap();
        menu.iter()
            .any(|entry| matches!(entry, MenuEntry::Item { title, .. } if title == "Finish setup…"))
    }

    /// A rehearsal's first run, up to the trial step.
    fn reach_the_trial(&self) {
        self.pick(MenuAction::OpenOnboarding);
        self.page_until("a microphone turned up", |page| {
            page.microphone_mark == Check::Passed
        });
        self.shell.ask_permission(0).unwrap();
        self.page_until("the answer showed", |page| page.can_continue);
        self.shell.setup_continue().unwrap();
        self.page_until("the keyboard step", |page| page.step == Step::Keys);
        self.shell.ask_permission(2).unwrap();
        self.shell.ask_permission(1).unwrap();
        self.page_until("both answers showed", |page| page.can_continue);
        self.shell.setup_continue().unwrap();
        self.page_until("the models step", |page| page.step == Step::Models);
        self.shell.setup_continue().unwrap();
        self.page_until("the trial step", |page| page.step == Step::Trial);
    }
}

#[test]
fn a_first_run_opens_by_itself_comes_forward_and_says_its_page_first() {
    let rig = Rig::rehearsal();

    rig.shell.watch_setup().unwrap();

    rig.wait_for(&ShellEvent::Raise { window: SETUP });
    let events = rig.seen();
    let place = |wanted: &ShellEvent| events.iter().position(|event| event == wanted);
    let page = events
        .iter()
        .position(|event| matches!(event, ShellEvent::Setup { .. }));
    let opened = place(&ShellEvent::OpenQuietly { window: SETUP });
    let raised = place(&ShellEvent::Raise { window: SETUP });
    assert!(page < opened, "the window opens with a page to show");
    assert!(opened < raised);

    let page = rig.page().unwrap();
    assert_eq!(page.step, Step::Microphone);
    assert_eq!(page.step_label, "Step 1 of 4");
    assert!(!page.can_go_back);
    assert_eq!(page.continue_label, "Continue");
}

#[test]
fn a_run_nobody_asked_to_watch_opens_nothing() {
    let rig = Rig::rehearsal();

    rig.settle();

    assert_eq!(rig.windows_said(), []);
    assert!(rig.offers_setup(), "the menu still leads to it");
}

#[test]
fn the_first_run_goes_through_every_step_and_finishing_is_stored() {
    let rig = Rig::rehearsal();
    rig.reach_the_trial();
    let page = rig.page().unwrap();
    assert_eq!(page.step_label, "Step 4 of 4");
    assert_eq!(page.continue_label, "Finish");
    assert!(!page.can_continue, "the trial has to pass");

    rig.shell.start_trial().unwrap();
    rig.page_until("it listened", |page| page.trial.can_stop);
    rig.shell.stop_trial().unwrap();
    let page = rig.page_until("it heard", |page| {
        page.trial.heard.is_some() && page.trial.phase == Phase::Idle
    });
    let heard = page.trial.heard.as_deref().unwrap();
    assert!(heard.starts_with(rehearsal::HEARD), "{heard}");
    let labels: Vec<_> = page.checks.iter().map(|row| row.label.as_str()).collect();
    assert_eq!(
        labels,
        ["Shortcut: failed", "Microphone: works", "Models: works"]
    );
    assert!(page.trial.problem, "the mouse says nothing about the keys");

    let press = PillState::ListeningPtt;
    rig.shell.stage(PillScene::State { state: press }).unwrap();
    let rest = PillState::Resting;
    rig.shell.stage(PillScene::State { state: rest }).unwrap();
    rig.page_until("the shortcut passed", |page| page.can_continue);
    rig.forget_events();

    rig.shell.setup_continue().unwrap();

    rig.wait_for(&ShellEvent::Close { window: SETUP });
    assert!(rig.finished_on_disk());
    assert_eq!(rig.page(), None);
    rig.swift_closed(SETUP);
    rig.settle();
    assert_eq!(rig.page(), None);
    assert!(!rig.offers_setup());
}

#[test]
fn opening_settings_during_onboarding_brings_onboarding_forward() {
    let rig = Rig::rehearsal();
    rig.pick(MenuAction::OpenOnboarding);
    rig.forget_events();

    rig.pick(MenuAction::OpenSettings);

    assert_eq!(rig.windows_said(), [ShellEvent::Raise { window: SETUP }]);
}

#[test]
fn a_walk_with_the_models_step_closes_settings_first() {
    let rig = Rig::rehearsal();
    rig.pick(MenuAction::OpenSettings);
    rig.forget_events();

    rig.pick(MenuAction::OpenOnboarding);

    let said = rig.windows_said();
    assert_eq!(
        said[..2],
        [
            ShellEvent::Close {
                window: AppWindow::Settings
            },
            ShellEvent::OpenQuietly { window: SETUP },
        ]
    );
}

#[test]
fn a_walk_without_the_models_step_opens_beside_settings() {
    let rig = Rig::on_a_fake_machine(true, NO_MICROPHONE);
    rig.pick(MenuAction::OpenSettings);
    rig.forget_events();

    rig.pick(MenuAction::OpenOnboarding);

    assert_eq!(
        rig.windows_said(),
        [
            ShellEvent::OpenQuietly { window: SETUP },
            ShellEvent::Raise { window: SETUP }
        ]
    );
}

#[test]
fn with_nothing_to_do_it_does_not_open() {
    let rig = Rig::on_a_fake_machine(true, GRANTED);

    rig.pick(MenuAction::OpenOnboarding);

    assert_eq!(rig.windows_said(), []);
    assert_eq!(rig.page(), None);
    assert!(!rig.offers_setup());
}

#[test]
fn a_window_the_user_closed_is_gone_and_settings_opens_again() {
    let rig = Rig::rehearsal();
    rig.pick(MenuAction::OpenOnboarding);
    rig.forget_events();

    rig.swift_closed(SETUP);
    rig.settle();

    assert_eq!(rig.page(), None);
    assert!(!rig.has_seen(&ShellEvent::Close { window: SETUP }));
    rig.pick(MenuAction::OpenSettings);
    assert_eq!(
        rig.windows_said(),
        [ShellEvent::Open {
            window: AppWindow::Settings
        }]
    );
}

/// Swift closes in order, so its report can land after the next window opened.
#[test]
fn the_late_report_of_our_own_close_leaves_a_new_window_alone() {
    let rig = Rig::on_a_fake_machine(true, NO_MICROPHONE);
    rig.pick(MenuAction::OpenOnboarding);
    rig.set_permissions(GRANTED);
    rig.page_until("the fix showed", |page| page.can_continue);
    rig.shell.setup_continue().unwrap();
    rig.wait_for(&ShellEvent::Close { window: SETUP });
    rig.set_permissions(NO_MICROPHONE);
    rig.pick(MenuAction::OpenOnboarding);
    rig.page_until("it opened again", |page| !page.can_continue);
    rig.forget_events();

    rig.swift_closed(SETUP);
    rig.settle();

    assert!(rig.page().is_some(), "the new window is still the walk's");
    assert_eq!(rig.windows_said(), []);

    rig.swift_closed(SETUP);
    rig.settle();
    assert_eq!(rig.page(), None, "this report is the user's");
}

#[test]
fn a_permission_lost_while_running_opens_it_without_the_focus() {
    let rig = Rig::on_a_fake_machine(true, GRANTED);
    rig.shell.watch_setup().unwrap();
    rig.settle();
    assert_eq!(rig.windows_said(), []);

    rig.set_permissions(NO_MICROPHONE);

    rig.wait_for(&ShellEvent::OpenQuietly { window: SETUP });
    rig.settle();
    assert_eq!(
        rig.windows_said(),
        [ShellEvent::OpenQuietly { window: SETUP }],
        "nothing brings it forward"
    );
}

#[test]
fn a_grant_shows_about_a_second_after_it_lands() {
    let rig = Rig::on_a_fake_machine(true, NO_MICROPHONE);
    rig.shell.watch_setup().unwrap();
    rig.page_until("a microphone turned up", |page| {
        page.microphone_mark == Check::Passed
    });

    rig.set_permissions(GRANTED);

    rig.page_within(Duration::from_millis(2500), "the grant showed", |page| {
        page.can_continue
    });
}

/// It looks every second, and only a change is worth crossing for.
#[test]
fn a_re_check_with_nothing_new_sends_nothing() {
    let rig = Rig::on_a_fake_machine(true, NO_MICROPHONE);
    rig.shell.watch_setup().unwrap();
    rig.page_until("a microphone turned up", |page| {
        page.microphone_mark == Check::Passed
    });
    thread::sleep(Duration::from_millis(100));
    let pages = |rig: &Rig| {
        let events = rig.seen().into_iter();
        events
            .filter(|event| matches!(event, ShellEvent::Setup { .. }))
            .count()
    };
    let before = pages(&rig);

    thread::sleep(Duration::from_millis(2200));

    assert_eq!(pages(&rig), before);
}

#[test]
fn a_broken_permission_offers_the_setup_from_the_menu() {
    let rig = Rig::on_a_fake_machine(true, NO_MICROPHONE);
    assert!(!rig.offers_setup(), "nothing has looked yet");

    rig.shell.watch_setup().unwrap();
    rig.settle();
    assert!(rig.offers_setup());
    rig.swift_closed(SETUP);
    rig.settle();
    assert!(rig.offers_setup(), "closing it fixes nothing");

    rig.set_permissions(GRANTED);
    let deadline = Instant::now() + WAIT;
    while rig.offers_setup() {
        assert!(Instant::now() < deadline, "the fix never showed");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn opening_again_in_the_same_run_starts_where_the_work_is() {
    let rig = Rig::rehearsal();
    rig.reach_the_trial();
    rig.swift_closed(SETUP);
    rig.settle();
    rig.forget_events();

    rig.pick(MenuAction::OpenOnboarding);

    let first = rig.seen().into_iter().find_map(|event| match event {
        ShellEvent::Setup { page } => Some(page.step),
        _ => None,
    });
    assert_eq!(first, Some(Step::Trial), "the pages know the hardware");
}

#[test]
fn a_trial_step_arms_the_shortcut_and_quitting_gives_it_back() {
    let rig = Rig::on_a_fake_machine(false, GRANTED);
    rig.pick(MenuAction::OpenOnboarding);
    let page = rig.page_until("the trial step", |page| page.step == Step::Trial);
    assert_eq!(page.trial.keys, Check::Waiting);
    let machine = rig.machine.as_ref().unwrap();
    assert!(machine.lock().unwrap().armed);

    now(rig.shell.quit()).unwrap();

    assert!(!machine.lock().unwrap().armed);
}

#[test]
fn the_flow_reaches_the_trial_only_through_an_open_window() {
    let rig = Rig::rehearsal();
    rig.reach_the_trial();
    rig.shell
        .stage(PillScene::State {
            state: PillState::ListeningPtt,
        })
        .unwrap();

    let page = rig.page_until("the shortcut was seen", |page| {
        page.trial.keys == Check::Passed
    });

    assert_eq!(page.trial.phase, Phase::Listening);
}
