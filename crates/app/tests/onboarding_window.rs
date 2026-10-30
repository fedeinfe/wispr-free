//! Onboarding through its real .slint UI on Slint's testing backend, with a fake machine standing
//! in for TCC, the key listener and the dictation flow, so that every state of every permission
//! can be reached without touching the real ones.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::fs;
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use i_slint_backend_testing::{
    AccessibleRole, ElementHandle, init_no_event_loop, mock_elapsed_time,
};
use slint::{ComponentHandle, LogicalSize, Model};
use wf_audio::InputDevice;
use wf_core::pill::PillState;
use wf_core::settings::{PipelineTier, Settings, SettingsStore};
use wf_core::shortcut::Platform;
use wf_dictation::{Event as FlowEvent, TrialPart};
use wf_models::{Catalog, Tier};
use wf_platform::keys::KeySignal;
use wf_platform::permissions::{Access, Permission, Permissions};
use wispr_free::onboarding::steps::Step;
use wispr_free::onboarding::{Machine, OnboardingHost, OnboardingParts, Opening};
use wispr_free::settings_window::{Desktop, Event, Mailbox, Paths, SettingsParts};
use wispr_free::{Consent, Mark, OnboardingWindow, Prefs, Setup, TrialPhase};

const GRANTED: Permissions = Permissions {
    microphone: Access::Granted,
    accessibility: Access::Granted,
    input_monitoring: Access::Granted,
};

const NOT_ASKED: Permissions = Permissions {
    microphone: Access::Unknown,
    accessibility: Access::Denied,
    input_monitoring: Access::Unknown,
};

struct FakeMachine {
    permissions: Cell<Permissions>,
    keys_listening: Cell<bool>,
    asked: RefCell<Vec<Permission>>,
    key_refreshes: RefCell<Vec<bool>>,
    armed: Cell<bool>,
    trials_started: Cell<u32>,
    trials_stopped: Cell<u32>,
    blocked: RefCell<Option<String>>,
    setup_needed: Cell<bool>,
    settings_closed: Cell<u32>,
    reopened: Cell<u32>,
}

impl FakeMachine {
    fn new(permissions: Permissions) -> Self {
        Self {
            permissions: Cell::new(permissions),
            keys_listening: Cell::new(true),
            asked: RefCell::default(),
            key_refreshes: RefCell::default(),
            armed: Cell::default(),
            trials_started: Cell::default(),
            trials_stopped: Cell::default(),
            blocked: RefCell::default(),
            setup_needed: Cell::default(),
            settings_closed: Cell::default(),
            reopened: Cell::default(),
        }
    }
}

impl Machine for FakeMachine {
    fn permissions(&self) -> Permissions {
        self.permissions.get()
    }

    fn ask(&self, permission: Permission) {
        self.asked.borrow_mut().push(permission);
    }

    fn keys_listening(&self) -> bool {
        self.keys_listening.get()
    }

    fn refresh_keys(&self, _permissions: &Permissions, may_prompt: bool) {
        self.key_refreshes.borrow_mut().push(may_prompt);
    }

    fn arm_trial(&self, armed: bool) {
        self.armed.set(armed);
    }

    fn start_trial(&self) {
        self.trials_started.set(self.trials_started.get() + 1);
    }

    fn stop_trial(&self) {
        self.trials_stopped.set(self.trials_stopped.get() + 1);
    }

    fn set_blocked(&self, reason: Option<String>) {
        *self.blocked.borrow_mut() = reason;
    }

    fn set_setup_needed(&self, needed: bool) {
        self.setup_needed.set(needed);
    }

    fn close_settings(&self) {
        self.settings_closed.set(self.settings_closed.get() + 1);
    }

    fn reopen_app(&self) {
        self.reopened.set(self.reopened.get() + 1);
    }
}

#[derive(Default)]
struct FakeDesktop {
    opened: RefCell<Vec<String>>,
    brought_to_front: Cell<u32>,
    mic_looks: Cell<u32>,
    probes_started: Cell<u32>,
}

impl Desktop for FakeDesktop {
    fn bring_to_front(&self) {
        self.brought_to_front.set(self.brought_to_front.get() + 1);
    }

    fn start_probes(&self, _mailbox: &Mailbox) {
        self.probes_started.set(self.probes_started.get() + 1);
    }

    fn set_launch_at_login(&self, _enabled: bool) -> io::Result<()> {
        Ok(())
    }

    fn set_dock_icon_visible(&self, _visible: bool) {}

    fn open(&self, target: &str) -> io::Result<()> {
        self.opened.borrow_mut().push(target.to_owned());
        Ok(())
    }

    fn capture_keys(
        &self,
        _on_signal: Box<dyn FnMut(KeySignal) + Send>,
    ) -> Result<Box<dyn Any>, String> {
        Ok(Box::new(()))
    }

    fn test_microphone(
        &self,
        _ranking: Vec<String>,
        _on_level: Box<dyn FnMut(f32) + Send>,
    ) -> Result<Box<dyn Any>, String> {
        Err("no mic here".to_owned())
    }

    fn pick_folder(&self, _start: &Path, _done: Box<dyn FnOnce(PathBuf)>) {}

    fn look_for_microphones(&self, _mailbox: &Mailbox) {
        self.mic_looks.set(self.mic_looks.get() + 1);
    }
}

struct Harness {
    dir: tempfile::TempDir,
    machine: Rc<FakeMachine>,
    desktop: Rc<FakeDesktop>,
    announced: Rc<Cell<u32>>,
    catalog: Arc<Catalog>,
    store: Arc<SettingsStore>,
    host: OnboardingHost,
}

impl Harness {
    fn new(permissions: Permissions) -> Self {
        Self::on(
            tempfile::tempdir().unwrap(),
            permissions,
            Catalog::bundled(),
        )
    }

    fn on(dir: tempfile::TempDir, permissions: Permissions, catalog: Catalog) -> Self {
        init_no_event_loop();
        let machine = Rc::new(FakeMachine::new(permissions));
        let desktop = Rc::new(FakeDesktop::default());
        let announced = Rc::new(Cell::new(0));
        let catalog = Arc::new(catalog);
        let store = Arc::new(SettingsStore::open(dir.path().join("settings.toml")));
        let host = OnboardingHost::new(Self::parts(
            &dir, &machine, &desktop, &announced, &catalog, &store,
        ));
        Self {
            dir,
            machine,
            desktop,
            announced,
            catalog,
            store,
            host,
        }
    }

    fn parts(
        dir: &tempfile::TempDir,
        machine: &Rc<FakeMachine>,
        desktop: &Rc<FakeDesktop>,
        announced: &Rc<Cell<u32>>,
        catalog: &Arc<Catalog>,
        store: &Arc<SettingsStore>,
    ) -> OnboardingParts {
        let announced = Rc::clone(announced);
        OnboardingParts {
            settings: SettingsParts {
                store: Arc::clone(store),
                catalog: Arc::clone(catalog),
                desktop: desktop.clone(),
                on_change: Rc::new(move |_: &Settings| announced.set(announced.get() + 1)),
                paths: Paths {
                    data_dir: dir.path().join("data"),
                    export_dir: dir.path().join("downloads"),
                },
                platform: Platform::MacOs,
            },
            machine: machine.clone(),
        }
    }

    /// A launch with what is on disk now. The machine keeps its state.
    fn relaunch(&mut self) {
        self.host.close();
        self.host = OnboardingHost::new(Self::parts(
            &self.dir,
            &self.machine,
            &self.desktop,
            &self.announced,
            &self.catalog,
            &self.store,
        ));
    }

    fn start(&self) {
        self.host.start().unwrap();
        self.settle_size();
    }

    fn settle_size(&self) {
        // Tall enough for every row: elements scrolled out of view can't be found.
        self.host.with_window(|open| {
            open.window()
                .window()
                .set_size(LogicalSize::new(760.0, 2600.0));
        });
    }

    fn with_window<T>(&self, use_it: impl FnOnce(&OnboardingWindow) -> T) -> T {
        self.host
            .with_window(|open| use_it(open.window()))
            .expect("the window is open")
    }

    fn setup<T>(&self, read: impl FnOnce(Setup<'_>) -> T) -> T {
        self.with_window(|window| read(window.global::<Setup>()))
    }

    fn prefs<T>(&self, read: impl FnOnce(Prefs<'_>) -> T) -> T {
        self.with_window(|window| read(window.global::<Prefs>()))
    }

    fn grant(&self) {
        self.machine.permissions.set(GRANTED);
        self.host.look();
    }

    fn set_permissions(&self, permissions: Permissions) {
        self.machine.permissions.set(permissions);
        self.host.look();
    }

    fn microphones(&self, names: &[&str]) {
        let devices = names
            .iter()
            .map(|&name| InputDevice {
                id: name.to_owned(),
                name: name.to_owned(),
            })
            .collect();
        self.host.with_window(|open| {
            open.pages().handle(Event::Microphones {
                devices,
                default: None,
            });
        });
        self.host.look();
    }

    fn element(&self, label: &str) -> ElementHandle {
        self.with_window(|window| {
            ElementHandle::find_by_accessible_label(window, label)
                .find(|element| element.accessible_role() != Some(AccessibleRole::Text))
                .unwrap_or_else(|| panic!("no control labelled {label:?}"))
        })
    }

    fn has(&self, label: &str) -> bool {
        self.with_window(|window| {
            ElementHandle::find_by_accessible_label(window, label)
                .any(|element| element.accessible_role() != Some(AccessibleRole::Text))
        })
    }

    fn shows_text(&self, text: &str) -> bool {
        self.with_window(|window| {
            ElementHandle::find_by_element_type_name(window, "Text").any(|element| {
                element
                    .accessible_label()
                    .is_some_and(|label| label.contains(text))
            })
        })
    }

    fn click(&self, label: &str) {
        self.element(label).invoke_accessible_default_action();
    }

    fn step(&self) -> Step {
        self.host
            .with_window(|open| open.step())
            .expect("the window is open")
    }

    fn steps(&self) -> Vec<Step> {
        self.host
            .with_window(|open| open.steps())
            .expect("the window is open")
    }

    fn can_continue(&self) -> bool {
        self.setup(|setup| setup.get_can_continue())
    }

    fn flow(&self, event: FlowEvent) {
        self.host.on_flow_event(&event);
    }

    /// Lets timers run, such as the one that closes the window.
    fn run_timers(&self) {
        mock_elapsed_time(Duration::from_millis(1));
    }

    fn saved(&self) -> Settings {
        SettingsStore::open(self.dir.path().join("settings.toml")).get()
    }

    fn install_models(&self, tier: Tier) {
        for model in self.catalog.models_for_tier(tier) {
            let folder = self.dir.path().join("data/models").join(&model.id);
            fs::create_dir_all(&folder).unwrap();
            for file in &model.files {
                fs::write(folder.join(&file.name), vec![0; file.size as usize]).unwrap();
            }
        }
    }

    /// The first run up to the trial step.
    fn reach_the_trial(&self) {
        self.install_models(Tier::Balanced);
        self.start();
        self.microphones(&["USB mic"]);
        assert_eq!(self.step(), Step::Trial);
    }

    /// A real shortcut, a voice and the words it heard.
    fn pass_the_trial(&self) {
        self.flow(FlowEvent::Pill(PillState::ListeningPtt));
        self.flow(FlowEvent::Level(0.5));
        self.flow(FlowEvent::Pill(PillState::Processing));
        self.flow(FlowEvent::Trial("Testing one two.".to_owned()));
        self.flow(FlowEvent::Pill(PillState::Resting));
    }

    fn finish(&self) {
        self.click("Finish");
        self.run_timers();
    }

    /// A user who went through it all, on a later launch.
    fn finished() -> Self {
        let harness = Self::new(GRANTED);
        harness.install_models(Tier::Balanced);
        harness
            .store
            .update(|settings| settings.onboarding.finished = true)
            .unwrap();
        harness
    }
}

#[test]
fn a_first_run_opens_on_the_microphone_in_front_and_looks_like_every_step() {
    let harness = Harness::new(NOT_ASKED);

    harness.start();

    assert_eq!(harness.steps(), Step::ALL);
    assert_eq!(harness.step(), Step::Microphone);
    assert_eq!(harness.desktop.brought_to_front.get(), 1);
    assert_eq!(harness.desktop.probes_started.get(), 1);
    assert_eq!(
        harness.setup(|setup| setup.get_microphone()),
        Consent::NotAsked
    );
    assert!(!harness.can_continue());
}

#[test]
fn a_finished_user_with_everything_working_never_sees_it() {
    let harness = Harness::finished();

    harness.start();

    assert!(!harness.host.is_open());
    assert!(!harness.machine.setup_needed.get());
    assert_eq!(*harness.machine.blocked.borrow(), None);
}

#[test]
fn allow_brings_up_the_os_question_and_the_answer_shows_without_a_restart() {
    let harness = Harness::new(NOT_ASKED);
    harness.start();

    harness.click("Allow Microphone access");
    assert_eq!(*harness.machine.asked.borrow(), [Permission::Microphone]);

    harness.microphones(&["USB mic"]);
    harness.set_permissions(Permissions {
        microphone: Access::Granted,
        ..NOT_ASKED
    });

    assert_eq!(
        harness.setup(|setup| setup.get_microphone()),
        Consent::Granted
    );
    assert!(harness.can_continue());
    assert!(!harness.has("Allow Microphone access"));
}

#[test]
fn a_denied_microphone_links_to_its_pane_since_the_os_wont_ask_twice() {
    let harness = Harness::new(Permissions {
        microphone: Access::Denied,
        ..NOT_ASKED
    });
    harness.start();

    assert!(!harness.has("Allow Microphone access"));
    harness.click("Open System Settings for Microphone access");

    let opened = harness.desktop.opened.borrow();
    assert_eq!(opened.len(), 1);
    assert!(opened[0].ends_with("Privacy_Microphone"), "{opened:?}");
}

#[test]
fn no_input_device_is_its_own_state_and_plugging_one_in_clears_it() {
    let harness = Harness::new(Permissions {
        microphone: Access::Granted,
        ..NOT_ASKED
    });
    harness.start();
    assert!(harness.shows_text("Looking for a microphone"));

    harness.microphones(&[]);

    assert!(harness.shows_text("No microphone connected"));
    assert!(harness.shows_text("Plug in a headset"));
    assert!(!harness.can_continue(), "nothing to dictate with");

    harness.microphones(&["AirPods"]);

    assert!(harness.shows_text("AirPods"));
    assert!(!harness.shows_text("No microphone connected"));
    assert!(harness.can_continue());
}

#[test]
fn the_page_keeps_looking_for_a_microphone_while_it_waits() {
    let harness = Harness::new(NOT_ASKED);
    harness.start();
    let before = harness.desktop.mic_looks.get();

    harness.host.look();
    harness.host.look();

    assert_eq!(harness.desktop.mic_looks.get(), before + 2);
}

#[test]
fn a_revoked_step_can_stop_the_user_going_on() {
    let harness = Harness::new(Permissions {
        microphone: Access::Granted,
        ..NOT_ASKED
    });
    harness.start();
    harness.microphones(&["USB mic"]);
    harness.click("Back");
    assert_eq!(harness.step(), Step::Microphone);
    assert!(harness.can_continue());

    harness.set_permissions(NOT_ASKED);

    assert!(!harness.can_continue());
}

#[test]
fn the_keyboard_step_needs_both_permissions_and_links_to_each_pane() {
    let harness = Harness::new(Permissions {
        microphone: Access::Granted,
        ..NOT_ASKED
    });
    harness.start();
    harness.microphones(&["USB mic"]);
    harness.click("Continue");
    assert_eq!(harness.step(), Step::Keys);

    harness.click("Allow Input Monitoring");
    harness.click("Allow Accessibility");
    assert_eq!(
        *harness.machine.asked.borrow(),
        [Permission::InputMonitoring, Permission::Accessibility]
    );
    harness.click("Open System Settings for Input Monitoring");
    harness.click("Open System Settings for Accessibility");
    let opened = harness.desktop.opened.borrow().clone();
    assert!(opened[0].ends_with("Privacy_ListenEvent"), "{opened:?}");
    assert!(opened[1].ends_with("Privacy_Accessibility"), "{opened:?}");

    harness.set_permissions(Permissions {
        microphone: Access::Granted,
        accessibility: Access::Granted,
        input_monitoring: Access::Denied,
    });
    assert!(!harness.can_continue());

    harness.grant();
    assert!(harness.can_continue());
}

#[test]
fn listening_to_keys_waits_for_the_keyboard_steps_own_question() {
    let harness = Harness::new(NOT_ASKED);
    harness.start();
    assert_eq!(harness.machine.key_refreshes.borrow().last(), Some(&false));

    harness.set_permissions(Permissions {
        input_monitoring: Access::Denied,
        ..NOT_ASKED
    });

    assert_eq!(harness.machine.key_refreshes.borrow().last(), Some(&true));
}

#[test]
fn allowed_keys_that_never_arrive_offer_a_reopen() {
    let harness = Harness::new(GRANTED);
    harness.install_models(Tier::Balanced);
    harness.machine.keys_listening.set(false);
    harness.start();
    harness.microphones(&["USB mic"]);
    harness.click("Back");
    harness.click("Back");
    assert_eq!(harness.step(), Step::Keys);
    assert!(!harness.setup(|setup| setup.get_keys_stuck()));

    for _ in 0..8 {
        harness.host.look();
    }

    assert!(harness.setup(|setup| setup.get_keys_stuck()));
    harness.click("Quit and reopen");
    assert_eq!(harness.machine.reopened.get(), 1);

    harness.machine.keys_listening.set(true);
    harness.host.look();
    assert!(!harness.setup(|setup| setup.get_keys_stuck()));
}

#[test]
fn the_models_step_lists_what_the_tier_needs_and_changes_with_it() {
    let harness = Harness::new(GRANTED);
    harness.start();
    harness.microphones(&["USB mic"]);
    assert_eq!(harness.step(), Step::Models);
    assert!(!harness.can_continue());

    let names = |harness: &Harness| {
        harness.prefs(|prefs| {
            let mut names: Vec<_> = prefs
                .get_models_in_use()
                .iter()
                .map(|row| row.name.to_string())
                .collect();
            names.sort();
            names
        })
    };
    let wanted = |harness: &Harness, tier| {
        let mut names: Vec<_> = harness
            .catalog
            .models_for_tier(tier)
            .iter()
            .map(|model| model.name.clone())
            .collect();
        names.sort();
        names
    };
    assert_eq!(names(&harness), wanted(&harness, Tier::Balanced));

    harness.prefs(|prefs| {
        let mut values = prefs.get_values();
        values.tier = 0;
        prefs.set_values(values);
        prefs.invoke_commit();
    });

    assert_eq!(harness.saved().pipeline.tier, PipelineTier::Lite);
    assert_eq!(names(&harness), wanted(&harness, Tier::Lite));
}

#[test]
fn the_recommended_tier_is_picked_for_someone_who_has_not_chosen() {
    let harness = Harness::new(GRANTED);
    harness.start();
    harness.microphones(&["USB mic"]);

    harness.host.with_window(|open| {
        open.pages().handle(Event::Hardware {
            summary: "8 GB".to_owned(),
            recommended: "Lite".to_owned(),
        });
    });
    harness.host.look();

    assert_eq!(harness.saved().pipeline.tier, PipelineTier::Lite);
    assert_eq!(harness.prefs(|prefs| prefs.get_values().tier), 0);
    assert!(harness.announced.get() > 0, "the running flow hears of it");

    harness.prefs(|prefs| {
        let mut values = prefs.get_values();
        values.tier = 2;
        prefs.set_values(values);
        prefs.invoke_commit();
    });
    harness.host.look();
    assert_eq!(
        harness.saved().pipeline.tier,
        PipelineTier::High,
        "the user's own pick stands"
    );
}

#[test]
fn an_unplugged_models_folder_reads_as_in_settings_and_comes_back() {
    let dir = tempfile::tempdir().unwrap();
    let drive = dir.path().join("drive");
    let harness = Harness::on(dir, GRANTED, Catalog::bundled());
    harness
        .store
        .update(|settings| settings.pipeline.models_folder = Some(drive.clone()))
        .unwrap();
    harness.start();
    harness.microphones(&["USB mic"]);

    assert_eq!(harness.step(), Step::Models);
    assert!(harness.prefs(|prefs| prefs.get_models_folder_missing()));
    assert!(harness.shows_text("This folder isn't available"));
    assert!(
        !harness
            .element("Download all")
            .accessible_enabled()
            .unwrap()
    );
    assert!(!harness.can_continue());
    assert!(!drive.exists(), "never recreated");

    fs::create_dir_all(&drive).unwrap();
    harness.host.look();

    assert!(!harness.prefs(|prefs| prefs.get_models_folder_missing()));
    assert!(
        harness
            .element("Download all")
            .accessible_enabled()
            .unwrap()
    );
}

/// Serves `hello` for every request, as many times as asked, and says what was requested.
fn serve_models(requests: usize) -> (u16, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let mut seen = Vec::new();
        for _ in 0..requests {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 2048];
            let read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]).into_owned();
            let line = request.lines().next().unwrap_or_default().to_owned();
            seen.push(line.split(' ').nth(1).unwrap_or_default().to_owned());
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
                )
                .unwrap();
        }
        seen
    });
    (port, server)
}

/// Every tier runs on the same three tiny models, served from `port`.
fn tiny_catalog(port: u16) -> Catalog {
    const HELLO_SHA256: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
    let tier = |name: &str| {
        format!(
            r#"[tiers.{name}]
denoise = false
vad = true
llm_cleanup = true
meeting_refine = false
diarization = false
vad_model = "vad"
asr_model = "asr"
llm_model = "llm"
"#
        )
    };
    let model = |id: &str, kind: &str, engine: &str| {
        format!(
            r#"[[models]]
id = "{id}"
name = "{id}"
kind = "{kind}"
engine = "{engine}"
license = "MIT"
languages = ["*"]
ram_mb = 1
[[models.files]]
name = "{id}.bin"
url = "http://127.0.0.1:{port}/{id}.bin"
size = 5
sha256 = "{HELLO_SHA256}"
"#
        )
    };
    let text = format!(
        "schema = 1\nversion = \"1\"\n{}{}{}{}{}{}",
        tier("lite"),
        tier("balanced"),
        tier("high"),
        model("vad", "vad", "sherpa-onnx"),
        model("asr", "asr", "sherpa-onnx"),
        model("llm", "llm", "llama.cpp"),
    );
    Catalog::from_toml(&text).unwrap()
}

/// Lets the download threads' events reach the window.
fn pump_until(harness: &Harness, what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "{what} didn't happen in time");
        harness.prefs(|prefs| prefs.invoke_events_arrived());
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn download_all_fetches_the_tier_one_at_a_time_through_the_settings_machinery() {
    let (port, server) = serve_models(3);
    let dir = tempfile::tempdir().unwrap();
    let harness = Harness::on(dir, GRANTED, tiny_catalog(port));
    harness.start();
    harness.microphones(&["USB mic"]);
    assert_eq!(harness.step(), Step::Models);
    assert!(harness.prefs(|prefs| !prefs.get_models_in_use_summary().is_empty()));

    harness.click("Download all");

    pump_until(&harness, "all three downloads", || {
        harness
            .prefs(|prefs| prefs.get_models_in_use_summary().is_empty() && !prefs.get_model_busy())
    });
    let mut requested = server.join().unwrap();
    requested.sort();
    assert_eq!(requested, ["/asr.bin", "/llm.bin", "/vad.bin"]);

    harness.host.look();
    assert!(harness.can_continue(), "the step sees what landed on disk");
}

#[test]
fn a_failed_download_is_reported_and_stops_the_queue() {
    // Nothing listens on this port.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let dir = tempfile::tempdir().unwrap();
    let harness = Harness::on(dir, GRANTED, tiny_catalog(port));
    harness.start();
    harness.microphones(&["USB mic"]);

    harness.click("Download all");

    pump_until(&harness, "the failure", || {
        harness.prefs(|prefs| !prefs.get_model_error().is_empty() && !prefs.get_model_busy())
    });
    assert!(!harness.can_continue());
    assert!(!harness.prefs(|prefs| prefs.get_models_in_use_summary().is_empty()));
}

#[test]
fn the_trial_listens_to_the_real_shortcut_only_while_its_step_shows() {
    let harness = Harness::new(GRANTED);
    harness.reach_the_trial();
    assert!(harness.machine.armed.get());
    assert!(
        harness
            .setup(|setup| setup.get_trial_message())
            .contains("Hold Fn")
    );

    harness.click("Back");
    assert!(!harness.machine.armed.get(), "dictation pastes again");

    harness.click("Continue");
    assert!(harness.machine.armed.get());
}

#[test]
fn closing_on_the_trial_step_gives_the_shortcut_back() {
    let harness = Harness::new(GRANTED);
    harness.reach_the_trial();
    assert!(harness.machine.armed.get());

    harness.host.close();

    assert!(!harness.machine.armed.get());
}

#[test]
fn a_passed_trial_shows_the_text_and_finishing_is_stored_for_good() {
    let mut harness = Harness::new(GRANTED);
    harness.reach_the_trial();
    assert!(!harness.can_continue());
    assert_eq!(harness.setup(|setup| setup.get_trial_keys()), Mark::Waiting);

    harness.flow(FlowEvent::Pill(PillState::ListeningPtt));
    assert_eq!(harness.setup(|setup| setup.get_trial_keys()), Mark::Passed);
    assert_eq!(
        harness.setup(|setup| setup.get_trial_phase()),
        TrialPhase::Listening
    );
    harness.flow(FlowEvent::Level(0.5));
    harness.flow(FlowEvent::Pill(PillState::Processing));
    assert_eq!(
        harness.setup(|setup| setup.get_trial_phase()),
        TrialPhase::Working
    );
    harness.flow(FlowEvent::Trial("Testing one two.".to_owned()));
    harness.flow(FlowEvent::Pill(PillState::Resting));

    assert!(harness.shows_text("Testing one two."));
    assert_eq!(
        harness.setup(|setup| setup.get_trial_microphone()),
        Mark::Passed
    );
    assert_eq!(
        harness.setup(|setup| setup.get_trial_models()),
        Mark::Passed
    );
    assert!(harness.can_continue());
    assert_eq!(harness.setup(|setup| setup.get_continue_label()), "Finish");

    harness.finish();

    assert!(!harness.host.is_open());
    assert!(harness.saved().onboarding.finished);
    assert!(!harness.machine.armed.get());

    harness.relaunch();
    harness.host.start().unwrap();
    assert!(!harness.host.is_open(), "never again");
}

#[test]
fn a_failed_trial_names_the_part_and_can_be_tried_again() {
    let harness = Harness::new(GRANTED);
    harness.reach_the_trial();

    harness.flow(FlowEvent::Pill(PillState::ListeningPtt));
    harness.flow(FlowEvent::TrialFailed {
        part: TrialPart::Microphone,
        reason: "No microphone found".to_owned(),
    });
    harness.flow(FlowEvent::Pill(PillState::Error {
        reason: "No microphone found".to_owned(),
    }));

    assert_eq!(
        harness.setup(|setup| setup.get_trial_microphone()),
        Mark::Failed
    );
    assert_eq!(harness.setup(|setup| setup.get_trial_keys()), Mark::Passed);
    assert_eq!(
        harness.setup(|setup| setup.get_trial_models()),
        Mark::Waiting
    );
    assert!(harness.setup(|setup| setup.get_trial_problem()));
    assert!(harness.shows_text("No microphone found"));
    assert!(!harness.can_continue());

    harness.pass_the_trial();
    assert!(harness.can_continue());
}

#[test]
fn trying_without_the_shortcut_uses_the_trial_call_and_blames_the_keys() {
    let harness = Harness::new(GRANTED);
    harness.reach_the_trial();

    harness.click("Try without the shortcut");
    assert_eq!(harness.machine.trials_started.get(), 1);

    harness.flow(FlowEvent::Pill(PillState::ListeningHandsFree));
    harness.click("Stop");
    assert_eq!(harness.machine.trials_stopped.get(), 1);

    harness.flow(FlowEvent::Level(0.5));
    harness.flow(FlowEvent::Pill(PillState::Processing));
    harness.flow(FlowEvent::Trial("Hello.".to_owned()));
    harness.flow(FlowEvent::Pill(PillState::Resting));

    assert_eq!(harness.setup(|setup| setup.get_trial_keys()), Mark::Failed);
    assert_eq!(
        harness.setup(|setup| setup.get_trial_models()),
        Mark::Passed
    );
    assert!(
        harness
            .setup(|setup| setup.get_trial_message())
            .contains("never arrived")
    );
    assert!(!harness.can_continue());
}

#[test]
fn the_trial_says_when_no_microphone_is_connected() {
    let harness = Harness::new(GRANTED);
    harness.reach_the_trial();

    harness.microphones(&[]);

    assert_eq!(
        harness.setup(|setup| setup.get_trial_microphone()),
        Mark::Failed
    );
    assert!(
        harness
            .setup(|setup| setup.get_trial_message())
            .starts_with("No microphone")
    );
}

#[test]
fn a_permission_revoked_while_running_brings_up_only_its_step_without_taking_focus() {
    let harness = Harness::finished();
    harness.start();
    assert!(!harness.host.is_open());

    harness.set_permissions(Permissions {
        input_monitoring: Access::Denied,
        ..GRANTED
    });

    assert!(harness.host.is_open());
    assert_eq!(harness.steps(), [Step::Keys]);
    assert_eq!(harness.desktop.brought_to_front.get(), 0);
    assert_eq!(harness.setup(|setup| setup.get_steps().row_count()), 1);
    assert!(harness.machine.setup_needed.get());
}

#[test]
fn a_revoked_permission_at_launch_comes_forward_with_only_its_step() {
    let harness = Harness::finished();
    harness.machine.permissions.set(Permissions {
        microphone: Access::Denied,
        ..GRANTED
    });

    harness.start();

    assert_eq!(harness.steps(), [Step::Microphone]);
    assert_eq!(harness.desktop.brought_to_front.get(), 1);
    assert_eq!(harness.setup(|setup| setup.get_continue_label()), "Done");
}

#[test]
fn fixing_the_step_is_done_and_changes_nothing_stored() {
    let harness = Harness::finished();
    harness.start();
    harness.set_permissions(Permissions {
        accessibility: Access::Denied,
        ..GRANTED
    });
    assert!(!harness.can_continue());

    harness.grant();
    assert!(harness.can_continue());
    harness.click("Done");
    harness.run_timers();

    assert!(!harness.host.is_open());
    assert!(harness.saved().onboarding.finished);
    assert!(!harness.machine.setup_needed.get());
}

#[test]
fn a_second_break_joins_the_steps_of_an_open_fix() {
    let harness = Harness::finished();
    harness.start();
    harness.set_permissions(Permissions {
        accessibility: Access::Denied,
        ..GRANTED
    });

    harness.set_permissions(Permissions {
        microphone: Access::Denied,
        accessibility: Access::Denied,
        ..GRANTED
    });

    assert_eq!(harness.steps(), [Step::Microphone, Step::Keys]);
    assert_eq!(harness.step(), Step::Keys, "stays where the user was");
}

#[test]
fn a_closed_fix_stays_closed_until_something_new_breaks() {
    let harness = Harness::finished();
    harness.start();
    let revoked = Permissions {
        accessibility: Access::Denied,
        ..GRANTED
    };
    harness.set_permissions(revoked);

    harness.host.close();
    harness.host.look();
    harness.host.look();
    assert!(!harness.host.is_open());

    harness.set_permissions(Permissions {
        microphone: Access::Denied,
        ..revoked
    });
    assert!(harness.host.is_open());
    assert_eq!(harness.steps(), [Step::Microphone, Step::Keys]);

    harness.host.close();
    harness.grant();
    assert!(!harness.host.is_open());
    harness.set_permissions(revoked);
    assert!(harness.host.is_open(), "fixed and broken again is new");
}

#[test]
fn the_pill_explains_what_is_missing_only_once_the_window_is_gone() {
    let harness = Harness::finished();
    harness.start();
    harness.set_permissions(Permissions {
        input_monitoring: Access::Denied,
        ..GRANTED
    });
    assert_eq!(
        *harness.machine.blocked.borrow(),
        None,
        "the window says it"
    );

    harness.host.close();
    let blocked = harness.machine.blocked.borrow().clone().unwrap();
    assert!(blocked.contains("Input Monitoring"), "{blocked}");

    harness.grant();
    assert_eq!(*harness.machine.blocked.borrow(), None);
}

#[test]
fn closing_the_first_run_keeps_it_for_the_next_launch_and_the_tray() {
    let mut harness = Harness::new(NOT_ASKED);
    harness.start();
    harness.host.close();

    assert!(!harness.host.is_open());
    assert!(harness.machine.setup_needed.get());
    assert!(!harness.saved().onboarding.finished);
    assert!(harness.machine.blocked.borrow().is_some());
    harness.host.look();
    assert!(
        !harness.host.is_open(),
        "only the user or a launch opens it"
    );

    harness.host.open(Opening::Foreground).unwrap();
    assert!(harness.host.is_open(), "the tray's way back");
    harness.host.close();

    harness.relaunch();
    harness.start();
    assert!(harness.host.is_open());
}

#[test]
fn a_later_launch_resumes_at_the_first_step_left_to_do() {
    let harness = Harness::new(Permissions {
        input_monitoring: Access::Denied,
        ..GRANTED
    });

    harness.start();
    assert_eq!(harness.step(), Step::Microphone, "until the first look");
    harness.microphones(&["USB mic"]);

    assert_eq!(harness.step(), Step::Keys);
    assert_eq!(harness.steps(), Step::ALL);
}

#[test]
fn a_later_launch_with_no_microphone_stays_on_that_step() {
    let harness = Harness::new(GRANTED);
    harness.install_models(Tier::Balanced);

    harness.start();
    harness.microphones(&[]);

    assert_eq!(harness.step(), Step::Microphone);
    assert!(harness.shows_text("No microphone connected"));
}

#[test]
fn a_late_look_at_the_hardware_leaves_a_user_who_moved_where_they_are() {
    let harness = Harness::new(GRANTED);
    harness.start();
    harness.microphones(&["USB mic"]);
    harness.click("Back");
    harness.click("Back");
    assert_eq!(harness.step(), Step::Microphone);

    harness.microphones(&["USB mic", "Headset"]);

    assert_eq!(harness.step(), Step::Microphone);
}

#[test]
fn a_chosen_model_gone_missing_brings_up_the_models_step_at_launch_only() {
    let harness = Harness::finished();
    harness.start();
    fs::remove_dir_all(harness.dir.path().join("data/models")).unwrap();

    harness.host.look();
    assert!(
        !harness.host.is_open(),
        "deleting a model isn't an emergency"
    );

    let later = {
        let mut harness = harness;
        harness.relaunch();
        harness
    };
    later.start();
    assert_eq!(later.steps(), [Step::Models]);
    assert_eq!(later.machine.settings_closed.get(), 1);
}

#[test]
fn the_window_is_freed_on_close() {
    let harness = Harness::new(NOT_ASKED);
    harness.start();
    let weak = harness.with_window(ComponentHandle::as_weak);
    assert!(weak.upgrade().is_some());

    harness.host.close();

    assert!(weak.upgrade().is_none());
}

#[test]
fn opening_twice_keeps_one_window() {
    let harness = Harness::new(NOT_ASKED);
    harness.start();
    let weak = harness.with_window(ComponentHandle::as_weak);

    harness.host.open(Opening::Foreground).unwrap();

    assert!(weak.upgrade().is_some());
    assert_eq!(harness.desktop.brought_to_front.get(), 2);
    assert!(harness.host.bring_forward());
}

#[test]
fn settings_finds_nothing_to_bring_forward_once_onboarding_is_gone() {
    let harness = Harness::finished();
    harness.start();
    assert!(!harness.host.bring_forward());
}
