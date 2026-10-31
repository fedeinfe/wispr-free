//! The Settings pages as Swift reaches them: through `Shell`, on their own thread, with what they
//! show coming back whole. What each page does is pinned in `settings_pages.rs`. This is what's
//! new: how it crosses, what follows what, and what outlives a window. Nothing here reaches the
//! network, the keys or the microphone.
#![cfg(target_os = "macos")]

use std::any::Any;
use std::fs;
use std::io;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use wf_core::settings::{PipelineTier, Replacement, Settings, SettingsStore};
use wf_core::shortcut::{Action, Key, KeyEvent, Side};
use wf_dictation::{Dictation, Event};
use wf_models::{Catalog, ModelKind};
use wf_platform::keys::KeySignal;
use wf_shell::app_windows::AppWindow;
use wf_shell::bridge::{EventSink, PillParts, SaveOutcome, Shell, ShellEvent};
use wf_shell::flow::{OnEvent, parts_at};
use wf_shell::menus::MenuAction;
use wf_shell::paths::Paths;
use wf_shell::settings::page::SettingsPage;
use wf_shell::settings::state::PagesState;
use wf_shell::settings::values::RuledChange;
use wf_shell::settings::{Desktop, Mailbox};

const WAIT: Duration = Duration::from_secs(10);

struct Recorder(Mutex<Sender<ShellEvent>>);

impl EventSink for Recorder {
    fn on_event(&self, event: ShellEvent) {
        let _ = self.0.lock().unwrap().send(event);
    }
}

type KeySink = Arc<Mutex<Option<Box<dyn FnMut(KeySignal) + Send>>>>;
type LevelSink = Arc<Mutex<Option<Box<dyn FnMut(f32) + Send>>>>;

/// What the pages asked of the machine, and a way for a test to answer.
#[derive(Clone, Default)]
struct Machine {
    keys: KeySink,
    levels: LevelSink,
    opened: Arc<Mutex<Vec<String>>>,
}

/// Gives the slot back when dropped, the way the real capture lets go.
struct Held<T>(Arc<Mutex<Option<T>>>);

impl<T> Drop for Held<T> {
    fn drop(&mut self) {
        self.0.lock().unwrap().take();
    }
}

struct FakeDesktop(Machine);

impl Desktop for FakeDesktop {
    fn bring_to_front(&self) {}

    fn start_probes(&self, _mailbox: &Mailbox) {}

    fn set_launch_at_login(&self, _enabled: bool) -> io::Result<()> {
        Ok(())
    }

    fn set_dock_icon_visible(&self, _visible: bool) {}

    fn open(&self, target: &str) -> io::Result<()> {
        self.0.opened.lock().unwrap().push(target.to_owned());
        Ok(())
    }

    fn capture_keys(
        &self,
        on_signal: Box<dyn FnMut(KeySignal) + Send>,
    ) -> Result<Box<dyn Any>, String> {
        *self.0.keys.lock().unwrap() = Some(on_signal);
        Ok(Box::new(Held(Arc::clone(&self.0.keys))))
    }

    fn test_microphone(
        &self,
        _ranking: Vec<String>,
        on_level: Box<dyn FnMut(f32) + Send>,
    ) -> Result<Box<dyn Any>, String> {
        *self.0.levels.lock().unwrap() = Some(on_level);
        Ok(Box::new(Held(Arc::clone(&self.0.levels))))
    }

    fn pick_folder(&self, _start: &Path, _done: Box<dyn FnOnce(PathBuf)>) {}
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
    machine: Machine,
    /// Sends the shell what the flow would.
    flow: OnEvent,
}

impl Rig {
    fn new() -> Self {
        Self::on(TempDir::new().unwrap())
    }

    /// A run whose settings name `folder` for its models, from the start, as a dev run's may.
    fn naming(folder: &Path) -> Self {
        let root = TempDir::new().unwrap();
        let settings = format!(
            "[pipeline]\nmodels_folder = {:?}\n",
            folder.to_str().unwrap()
        );
        fs::write(root.path().join("settings.toml"), settings).unwrap();
        Self::on(root)
    }

    fn on(folder: TempDir) -> Self {
        let paths = Paths::under(folder.path(), "/nowhere/wf-engine-host".into());
        let (sender, events) = mpsc::channel();
        let sink = Arc::new(Recorder(Mutex::new(sender)));
        let store = SettingsStore::open(&paths.settings);
        let pill = PillParts {
            prefs: paths.pill.clone(),
            play: Box::new(|_| {}),
            microphones: Box::new(Vec::new),
        };
        let machine = Machine::default();
        let desktop = machine.clone();
        let mut flow = None;
        let shell = Shell::wired_with(
            store,
            sink,
            pill,
            paths.clone(),
            Some(Box::new(move || Rc::new(FakeDesktop(desktop)))),
            |settings, on_event| {
                flow = Some(Arc::clone(&on_event));
                Dictation::start(parts_at(&paths, settings, on_event))
            },
        );
        Self {
            folder,
            shell,
            events,
            machine,
            flow: flow.unwrap(),
        }
    }

    fn pages(&self) -> PagesState {
        now(self.shell.pages_state()).unwrap().unwrap()
    }

    fn settings(&self) -> Settings {
        self.pages();
        self.shell.settings().unwrap().settings
    }

    fn saved(&self) -> Settings {
        self.pages();
        SettingsStore::open(self.folder.path().join("settings.toml")).get()
    }

    fn drain(&self) -> Vec<ShellEvent> {
        self.events.try_iter().collect()
    }

    fn press(&self, key: Key) {
        self.send_key(KeyEvent::press(key));
    }

    fn release(&self, key: Key) {
        self.send_key(KeyEvent::release(key));
    }

    fn send_key(&self, event: KeyEvent) {
        let mut keys = self.machine.keys.lock().unwrap();
        (keys.as_mut().expect("capturing keys"))(KeySignal::Key(event));
    }

    /// Until `done`, for what runs on a thread of its own.
    fn wait_for(&self, done: impl Fn(&PagesState) -> bool) -> PagesState {
        let deadline = Instant::now() + WAIT;
        loop {
            let state = self.pages();
            if done(&state) {
                return state;
            }
            assert!(Instant::now() < deadline, "never happened: {state:?}");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn default_models(&self) -> PathBuf {
        self.folder.path().join("models")
    }
}

/// The smallest model, as a sparse file, so a copy of it costs nothing. Says where and which.
fn lay_out_model(root: &Path) -> (i32, String) {
    let catalog = Catalog::bundled();
    let is_vad = |model: &&wf_models::ModelEntry| model.kind == ModelKind::Vad;
    let model = catalog.models.iter().find(is_vad).unwrap();
    let dir = root.join(&model.id);
    fs::create_dir_all(&dir).unwrap();
    for file in &model.files {
        let handle = fs::File::create(dir.join(&file.name)).unwrap();
        handle.set_len(file.size).unwrap();
    }
    let index = catalog.models.iter().position(|known| known.id == model.id);
    (index.unwrap() as i32, model.id.clone())
}

#[test]
fn settings_has_its_pages_before_it_opens() {
    let rig = Rig::new();

    now(rig.shell.menu_action(MenuAction::OpenSettings)).unwrap();

    let events = rig.drain();
    let place = |wanted: fn(&ShellEvent) -> bool| events.iter().position(wanted);
    let filled = place(
        |event| matches!(event, ShellEvent::Pages { state } if state.shortcuts.len() == Action::ALL.len()),
    );
    let opened = place(|event| {
        matches!(
            event,
            ShellEvent::Open {
                window: AppWindow::Settings
            }
        )
    });
    assert!(filled.unwrap() < opened.unwrap(), "{events:?}");
}

#[test]
fn a_tier_goes_through_its_rule_and_a_stage_makes_it_custom() {
    let rig = Rig::new();

    rig.shell
        .change_setting(RuledChange::Tier {
            tier: PipelineTier::Lite,
        })
        .unwrap();

    let preset = Catalog::bundled().preset(wf_models::Tier::Lite).clone();
    let pipeline = rig.saved().pipeline;
    assert_eq!(pipeline.tier, PipelineTier::Lite);
    assert_eq!(pipeline.llm_cleanup, preset.llm_cleanup);
    assert_eq!(pipeline.asr_model, None);

    rig.shell
        .change_setting(RuledChange::Denoise {
            on: !pipeline.denoise,
        })
        .unwrap();

    let pipeline = rig.saved().pipeline;
    assert_eq!(pipeline.tier, PipelineTier::Custom);
    assert_eq!(
        pipeline.asr_model.as_deref(),
        Some(preset.asr_model.as_str())
    );
}

#[test]
fn a_model_picked_from_a_menu_makes_the_tier_custom_and_shows_as_selected() {
    let rig = Rig::new();
    let menu = rig.pages().asr_menu;
    let other = i32::from(menu.selected == 0);

    rig.shell
        .change_setting(RuledChange::AsrModel { option: other })
        .unwrap();

    let state = rig.pages();
    assert_eq!(state.asr_menu.selected, other);
    assert_eq!(rig.saved().pipeline.tier, PipelineTier::Custom);
    assert!(state.asr_menu.model.is_some());
}

/// Each side writes in a tight loop. Swift's retries as `edit` would, and neither may lose a value.
#[test]
fn a_save_and_a_rule_racing_lose_nothing() {
    let rig = Rig::new();
    rig.pages();
    let shell = Arc::clone(&rig.shell);
    let rounds = 60;

    let saver = thread::spawn(move || {
        for round in 0..rounds {
            loop {
                let latest = shell.settings().unwrap();
                let mut settings = latest.settings;
                settings.style.dictionary.push(format!("word{round}"));
                let saved = now(Arc::clone(&shell).save(settings, latest.revision)).unwrap();
                if matches!(saved, SaveOutcome::Saved { .. }) {
                    break;
                }
            }
        }
    });
    for round in 0..rounds {
        rig.shell
            .change_setting(RuledChange::Denoise { on: round % 2 == 0 })
            .unwrap();
        rig.shell.add_word(format!("rule{round}")).unwrap();
    }
    saver.join().unwrap();

    let saved = rig.saved();
    assert!(!saved.pipeline.denoise);
    assert_eq!(saved.style.dictionary.len(), rounds * 2);
}

#[test]
fn the_pages_follow_what_changes_elsewhere_and_keep_what_is_being_typed() {
    let rig = Rig::new();
    rig.shell.add_replacement().unwrap();
    rig.shell
        .edit_replacement(
            0,
            Replacement {
                wrong: String::new(),
                right: "half typed".to_owned(),
            },
        )
        .unwrap();
    rig.pages();

    (rig.flow)(Event::WordLearned("Federico".to_owned()));
    now(rig.shell.menu_action(MenuAction::Language {
        code: Some("it".to_owned()),
    }))
    .unwrap();

    let state = rig.wait_for(|state| !state.dictionary.is_empty());
    assert_eq!(state.dictionary, ["Federico"]);
    let state = rig.wait_for(|state| !state.languages_auto);
    assert!(
        state
            .languages
            .iter()
            .any(|row| row.name == "Italian" && row.selected)
    );
    assert_eq!(state.replacements[0].right, "half typed");
}

#[test]
fn a_shortcut_recorded_through_the_shell_is_stored() {
    let rig = Rig::new();
    let row = Action::ALL
        .iter()
        .position(|a| *a == Action::PasteLast)
        .unwrap();

    rig.shell.record_shortcut(row as i32).unwrap();
    let state = rig.pages();
    assert_eq!(state.recording_action, Some(row as u32));
    assert_eq!(state.recording_keys, "Press keys…");

    let keys = [
        Key::Ctrl(Side::Either),
        Key::Alt(Side::Either),
        Key::Char('K'),
    ];
    keys.iter().for_each(|key| rig.press(*key));
    assert_ne!(rig.pages().recording_keys, "Press keys…");
    keys.iter().for_each(|key| rig.release(*key));

    let state = rig.pages();
    assert_eq!(state.recording_action, None);
    assert!(state.shortcuts[row].combo.contains('K'));
    assert!(
        rig.saved()
            .shortcuts
            .paste_last
            .keys()
            .contains(&Key::Char('K'))
    );
}

#[test]
fn closing_settings_gives_back_the_keys_and_the_microphone() {
    let rig = Rig::new();
    rig.shell.record_shortcut(0).unwrap();
    rig.shell.toggle_microphone_test().unwrap();
    let state = rig.pages();
    assert!(state.microphone_testing && state.recording_action.is_some());

    rig.shell.window_closed(AppWindow::Settings).unwrap();

    let state = rig.pages();
    assert!(!state.microphone_testing);
    assert_eq!(state.recording_action, None);
    assert!(rig.machine.keys.lock().unwrap().is_none());
    assert!(rig.machine.levels.lock().unwrap().is_none());
}

#[test]
fn the_microphone_test_crosses_as_levels_only_while_it_runs() {
    let rig = Rig::new();
    rig.shell.toggle_microphone_test().unwrap();
    rig.pages();
    rig.drain();

    (rig.machine.levels.lock().unwrap().as_mut().unwrap())(0.5);

    rig.pages();
    let events = rig.drain();
    assert!(
        events.contains(&ShellEvent::Level { value: 0.5 }),
        "{events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ShellEvent::Pages { .. }))
    );
}

/// The spec's reason for moving jobs out of the windows.
#[test]
fn a_move_outlives_the_window_that_started_it() {
    let rig = Rig::new();
    let (_, id) = lay_out_model(&rig.default_models());
    let target = rig.folder.path().join("elsewhere");
    fs::create_dir_all(&target).unwrap();
    rig.shell.recheck_models_folder().unwrap();
    rig.shell.folder_picked(target.clone()).unwrap();
    assert!(!rig.pages().pending_folder.is_empty());

    rig.shell.move_folder().unwrap();
    rig.shell.window_closed(AppWindow::Settings).unwrap();

    rig.wait_for(|state| !state.moving_models && state.models_folder == target.to_str().unwrap());
    assert_eq!(rig.saved().pipeline.models_folder, Some(target.clone()));
    assert!(target.join(&id).is_dir());
    assert!(!rig.default_models().join(&id).exists());
}

#[test]
fn a_chosen_folder_comes_back_from_the_front_end_and_applies() {
    let rig = Rig::new();
    let target = rig.folder.path().join("elsewhere");
    fs::create_dir_all(&target).unwrap();
    rig.pages();
    rig.drain();

    rig.shell.choose_models_folder().unwrap();
    rig.shell.folder_picked(target.clone()).unwrap();

    assert_eq!(rig.pages().models_folder, target.to_str().unwrap());
}

#[test]
fn inside_its_own_folder_a_run_may_change_models() {
    let rig = Rig::new();
    let (index, _) = lay_out_model(&rig.default_models());
    rig.shell.recheck_models_folder().unwrap();
    assert!(!rig.pages().models_read_only);

    rig.shell.delete_model(index).unwrap();

    let state = rig.pages();
    assert_eq!(state.model_error, "");
    assert!(!state.models[index as usize].installed);
}

#[test]
fn a_named_folder_outside_the_run_can_be_read_but_not_changed() {
    let outside = TempDir::new().unwrap();
    let (index, id) = lay_out_model(outside.path());
    let rig = Rig::naming(outside.path());

    let state = rig.pages();
    assert!(state.models_read_only);
    assert!(state.models[index as usize].installed, "it can be read");

    rig.shell.delete_model(index).unwrap();
    assert!(rig.pages().model_error.contains("can only be read"));
    assert!(outside.path().join(&id).is_dir());

    let missing = state
        .models
        .iter()
        .find(|row| !row.installed)
        .unwrap()
        .index;
    rig.shell.download(missing).unwrap();
    let said = rig.wait_for(|state| !state.model_busy).model_error;
    assert!(said.contains("can only be read"), "{said}");
    let missing_id = &Catalog::bundled().models[missing as usize].id;
    assert!(!outside.path().join(missing_id).exists());

    rig.shell.delete_all().unwrap();
    assert!(rig.pages().data_status.contains("couldn't be removed"));
    assert!(outside.path().join(&id).is_dir());
}

#[test]
fn a_link_inside_the_run_does_not_get_past_the_fence() {
    let outside = TempDir::new().unwrap();
    let (index, id) = lay_out_model(outside.path());
    let root = TempDir::new().unwrap();
    let link = root.path().join("linked");
    symlink(outside.path(), &link).unwrap();
    let settings = format!("[pipeline]\nmodels_folder = {:?}\n", link.to_str().unwrap());
    fs::write(root.path().join("settings.toml"), settings).unwrap();
    let rig = Rig::on(root);
    assert!(rig.pages().models_read_only);

    rig.shell.delete_model(index).unwrap();

    assert!(outside.path().join(&id).is_dir());
}

#[test]
fn a_model_folder_linked_out_of_the_default_folder_stays_put() {
    let outside = TempDir::new().unwrap();
    let (index, id) = lay_out_model(outside.path());
    let rig = Rig::new();
    fs::create_dir_all(rig.default_models()).unwrap();
    symlink(outside.path().join(&id), rig.default_models().join(&id)).unwrap();
    rig.shell.recheck_models_folder().unwrap();

    rig.shell.delete_model(index).unwrap();

    assert!(rig.pages().model_error.contains("can only be read"));
    assert!(outside.path().join(&id).is_dir());
}

#[test]
fn a_notice_stays_with_the_page_that_raised_it() {
    let folder = TempDir::new().unwrap();
    let path = folder.path().join("settings.toml");
    fs::write(&path, "not toml [").unwrap();
    let rig = Rig::on(folder);
    now(rig.shell.set_settings_page(SettingsPage::Privacy)).unwrap();

    let state = rig.pages();
    assert!(state.notice.contains("damaged"));
    assert_eq!(state.notice_page, Some(SettingsPage::Privacy));

    now(rig.shell.set_settings_page(SettingsPage::Pipeline)).unwrap();
    assert_eq!(rig.pages().notice_page, Some(SettingsPage::Privacy));

    rig.shell
        .change_setting(RuledChange::Denoise { on: true })
        .unwrap();
    let state = rig.pages();
    assert_eq!(state.notice, "");
    assert_eq!(state.notice_page, None);
}

#[test]
fn a_stale_index_does_not_take_the_pages_down() {
    let rig = Rig::new();

    rig.shell.remove_replacement(9).unwrap();
    rig.shell.remove_snippet(9).unwrap();
    rig.shell.verify(9999).unwrap();
    rig.shell.delete_model(9999).unwrap();
    rig.shell.add_word("still here".to_owned()).unwrap();

    assert_eq!(rig.pages().dictionary, ["still here"]);
}

#[test]
fn opening_a_folder_and_updates_go_to_the_machine() {
    let rig = Rig::new();

    rig.shell.open_data_folder().unwrap();
    rig.shell.check_for_updates().unwrap();
    rig.pages();

    let opened = rig.machine.opened.lock().unwrap();
    assert_eq!(opened[0], rig.folder.path().to_str().unwrap());
    assert!(opened[1].starts_with("https://"));
}

#[test]
fn delete_all_leaves_a_run_on_the_defaults_and_says_so() {
    let rig = Rig::new();
    rig.shell.add_word("gone".to_owned()).unwrap();
    rig.pages();

    rig.shell.delete_all().unwrap();

    let state = rig.pages();
    assert_eq!(state.data_status, "All data deleted.");
    assert!(state.dictionary.is_empty());
    assert_eq!(rig.settings(), Settings::default());
}

#[test]
fn quitting_lets_the_pages_go() {
    let rig = Rig::new();
    rig.pages();

    now(rig.shell.quit()).unwrap();

    assert_eq!(now(rig.shell.pages_state()).unwrap(), None);
}
