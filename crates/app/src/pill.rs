//! Connects the pure pill model to the Slint window and the native overlay.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::winit_030::winit::event_loop::EventLoop;
use slint::winit_030::winit::window::{Window as WinitWindow, WindowAttributes};
use slint::winit_030::{EventLoopBuilder, WinitWindowAccessor};
use slint::{
    ComponentHandle, LogicalPosition, Model, ModelRc, PhysicalPosition, PlatformError,
    SharedString, Timer, TimerMode, VecModel, WindowPosition,
};
use wf_audio::AudioBackend as _;
use wf_audio::{CpalBackend, sounds};
use wf_core::pill::dock::{Display, Point};
use wf_core::pill::waveform::{BARS, Waveform};
use wf_core::pill::{Pill, PillCommand, PillState, Sound, Tick};
use wf_platform::overlay;

use crate::prefs::Prefs;
use crate::{PillLook, PillWindow};

/// Poll rates: animating, watching the cursor, and hidden.
const FRAME_POLL: Duration = Duration::from_millis(16);
const CURSOR_POLL: Duration = Duration::from_millis(40);
const HIDDEN_POLL: Duration = Duration::from_millis(250);
/// How often to re-read the screens and re-assert the stacking order.
const DISPLAY_POLL: Duration = Duration::from_secs(1);
const HIDE_DURATION: Duration = Duration::from_secs(60 * 60);
const TOAST_DURATION: Duration = Duration::from_secs(4);
/// How long a question stays up when nobody answers it.
const CARD_SHOWN: Duration = Duration::from_secs(30);
const MENU_LABEL_CHARS: usize = 48;

/// The Language menu, with the code each entry stands for. Auto-detect has none.
pub const LANGUAGES: [(&str, Option<&str>); 3] = [
    ("Auto-detect", None),
    ("English", Some("en")),
    ("Italiano", Some("it")),
];
const PIPELINE_QUALITIES: [&str; 4] = ["Lite", "Balanced", "High", "Custom"];

/// What the user asked for through the pill. Moving and hiding it are handled internally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PillEvent {
    ToggleHandsFree,
    Cancel,
    Stop,
    PasteLast,
    PasteRecent(usize),
    SelectMicrophone(usize),
    SelectLanguage(usize),
    SelectPipelineQuality(usize),
    TakeNotes,
    OpenMeetings,
    OpenSettings,
    Quit,
}

type EventHandler = Rc<dyn Fn(PillEvent)>;

thread_local! {
    /// True while the pill window is being created, for the backend-wide attributes hook.
    static CREATING_OVERLAY: Cell<bool> = const { Cell::new(false) };
}

/// Picks the windowing backend. Must run before any window exists.
pub fn select_backend() -> Result<(), PlatformError> {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .with_winit_event_loop_builder(event_loop_builder())
        .with_winit_window_attributes_hook(overlay_attributes)
        .select()
}

fn event_loop_builder() -> EventLoopBuilder {
    #[allow(unused_mut)]
    let mut builder = EventLoop::with_user_event();
    // The pill must not pull focus from the app being typed into, starting with launch.
    #[cfg(target_os = "macos")]
    {
        use slint::winit_030::winit::platform::macos::{
            ActivationPolicy, EventLoopBuilderExtMacOS,
        };
        builder
            .with_activation_policy(ActivationPolicy::Accessory)
            .with_activate_ignoring_other_apps(false);
    }
    // winit has no layer-shell, and override-redirect and global cursor polling need X11.
    #[cfg(target_os = "linux")]
    if std::env::var_os("DISPLAY").is_some() {
        use slint::winit_030::winit::platform::x11::EventLoopBuilderExtX11;
        builder.with_x11();
    }
    builder
}

fn overlay_attributes(attributes: WindowAttributes) -> WindowAttributes {
    if !CREATING_OVERLAY.get() {
        return attributes;
    }
    let attributes = attributes.with_active(false);
    #[cfg(target_os = "linux")]
    let attributes = {
        use slint::winit_030::winit::platform::x11::WindowAttributesExtX11;
        attributes.with_override_redirect(true)
    };
    attributes
}

/// Handle to the pill, cheap to clone into callbacks.
#[derive(Clone)]
pub struct PillController {
    inner: Rc<Inner>,
}

struct Inner {
    ui: PillWindow,
    bars: Rc<VecModel<f32>>,
    timer: Timer,
    core: RefCell<Core>,
    handler: RefCell<Option<EventHandler>>,
    prefs_path: Option<PathBuf>,
}

struct Core {
    pill: Pill,
    waveform: Waveform,
    prefs: Prefs,
    toast: Option<(String, Instant)>,
    card_until: Option<Instant>,
    displays: Vec<Display>,
    started: Instant,
    last_poll: Instant,
    next_display_poll: Instant,
    overlay_applied: bool,
    hit_test: Option<bool>,
    poll: Duration,
}

impl PillController {
    pub fn new(prefs: Prefs, prefs_path: Option<PathBuf>) -> Result<Self, PlatformError> {
        CREATING_OVERLAY.set(true);
        let ui = PillWindow::new();
        CREATING_OVERLAY.set(false);
        let ui = ui?;

        let bars = Rc::new(VecModel::from(vec![0.0; BARS]));
        ui.set_bars(ModelRc::from(bars.clone()));
        ui.set_languages(string_model(LANGUAGES.map(|(name, _)| name)));
        ui.set_pipeline_qualities(string_model(PIPELINE_QUALITIES));

        let now = Instant::now();
        let controller = Self {
            inner: Rc::new(Inner {
                ui,
                bars,
                timer: Timer::default(),
                core: RefCell::new(Core {
                    pill: Pill::new(prefs.edge),
                    waveform: Waveform::new(),
                    prefs,
                    toast: None,
                    card_until: None,
                    displays: Vec::new(),
                    started: now,
                    last_poll: now,
                    next_display_poll: now,
                    overlay_applied: false,
                    hit_test: None,
                    poll: CURSOR_POLL,
                }),
                handler: RefCell::new(None),
                prefs_path,
            }),
        };
        controller.connect_ui();
        controller.load_microphones();
        Ok(controller)
    }

    /// Shows the window and starts polling. The native overlay is applied on the first poll with
    /// a live window.
    pub fn show(&self) -> Result<(), PlatformError> {
        self.inner.ui.show()?;
        let this = self.clone();
        self.inner
            .timer
            .start(TimerMode::Repeated, CURSOR_POLL, move || this.poll());
        Ok(())
    }

    /// Routes user requests to `handler`. It may call back into this controller.
    pub fn on_event(&self, handler: impl Fn(PillEvent) + 'static) {
        *self.inner.handler.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn state(&self) -> PillState {
        self.inner.core.borrow().pill.state().clone()
    }

    pub fn set_state(&self, state: PillState) {
        let mut core = self.inner.core.borrow_mut();
        if state.bar_scale().is_none() {
            core.waveform.reset();
        }
        let cue = core.pill.set_state(state);
        if let Some(sound) = cue.filter(|_| core.prefs.sounds) {
            sounds::play(sound);
        }
    }

    /// The Sounds setting.
    pub fn set_sounds(&self, sounds: bool) {
        self.inner.core.borrow_mut().prefs.sounds = sounds;
    }

    /// The Show pill setting.
    pub fn set_shown(&self, shown: bool) {
        self.inner.core.borrow_mut().pill.set_turned_off(!shown);
    }

    /// A short message by the pill. It shows even with the pill hidden, because the text it is
    /// about went somewhere the user can't see.
    pub fn show_toast(&self, text: String) {
        let until = Instant::now() + TOAST_DURATION;
        self.inner.core.borrow_mut().toast = Some((text, until));
    }

    /// The Recent transcripts menu.
    /// Asks whether to take notes for a meeting in `app`. The card goes away by itself.
    pub fn suggest_meeting(&self, app: String) {
        self.inner.ui.set_card_app(app.into());
        let mut core = self.inner.core.borrow_mut();
        core.pill.set_card(true);
        core.card_until = Some(Instant::now() + CARD_SHOWN);
    }

    pub fn hide_card(&self) {
        let mut core = self.inner.core.borrow_mut();
        core.pill.set_card(false);
        core.card_until = None;
    }

    pub fn set_recent(&self, transcripts: &[String]) {
        let labels = transcripts.iter().map(|text| menu_label(text));
        let labels: Vec<SharedString> = labels.collect();
        let labels = ModelRc::from(Rc::new(VecModel::from(labels)));
        self.inner.ui.set_recent_transcripts(labels);
    }

    /// The id of the device at `index` of the Microphone menu.
    pub fn microphone_id(&self, index: usize) -> Option<String> {
        let id = self.inner.ui.get_microphone_ids().row_data(index)?;
        Some(id.to_string())
    }

    /// A weak handle other threads can use to reach the UI thread.
    pub fn window(&self) -> slint::Weak<PillWindow> {
        self.inner.ui.as_weak()
    }

    /// Latest level from the meter, 0.0..=1.0.
    pub fn set_level(&self, level: f32) {
        self.inner.core.borrow_mut().waveform.set_level(level);
    }

    pub fn play_sound(&self, sound: Sound) {
        sounds::play(sound);
    }

    fn connect_ui(&self) {
        let ui = &self.inner.ui;
        let forward = |event: PillEvent| {
            let this = self.clone();
            move || this.emit(event.clone())
        };
        let forward_index = |event: fn(usize) -> PillEvent| {
            let this = self.clone();
            move |index: i32| {
                if let Ok(index) = usize::try_from(index) {
                    this.emit(event(index));
                }
            }
        };

        let this = self.clone();
        ui.on_pill_pressed(move || this.inner.core.borrow_mut().pill.press());
        let this = self.clone();
        ui.on_pill_released(move || this.released());
        let this = self.clone();
        ui.on_hide_for_an_hour(move || this.inner.core.borrow_mut().pill.hide_for(HIDE_DURATION));

        ui.on_cancel_clicked(forward(PillEvent::Cancel));
        let this = self.clone();
        ui.on_take_notes(move || {
            this.hide_card();
            this.emit(PillEvent::TakeNotes);
        });
        let this = self.clone();
        ui.on_decline_notes(move || this.hide_card());
        ui.on_stop_clicked(forward(PillEvent::Stop));
        ui.on_paste_last(forward(PillEvent::PasteLast));
        ui.on_open_meetings(forward(PillEvent::OpenMeetings));
        ui.on_open_settings(forward(PillEvent::OpenSettings));
        ui.on_quit(forward(PillEvent::Quit));
        ui.on_paste_recent(forward_index(PillEvent::PasteRecent));
        ui.on_select_microphone(forward_index(PillEvent::SelectMicrophone));
        ui.on_select_language(forward_index(PillEvent::SelectLanguage));
        ui.on_select_pipeline_quality(forward_index(PillEvent::SelectPipelineQuality));
    }

    /// `handler` runs on the UI thread whenever another thread calls `invoke_events_arrived`.
    pub fn on_events_arrived(&self, handler: impl FnMut() + 'static) {
        self.inner.ui.on_events_arrived(handler);
    }

    /// Fills the Microphone submenu without blocking the UI on device enumeration.
    fn load_microphones(&self) {
        let ui = self.inner.ui.as_weak();
        let spawned = std::thread::Builder::new()
            .name("pill-microphones".into())
            .spawn(move || {
                let devices = CpalBackend::new().devices();
                let _ = ui.upgrade_in_event_loop(move |ui| {
                    let names = devices.iter().map(|device| device.name.as_str());
                    ui.set_microphones(string_model(names));
                    let ids = devices.iter().map(|device| device.id.as_str());
                    ui.set_microphone_ids(string_model(ids));
                });
            });
        if let Err(err) = spawned {
            eprintln!("could not list microphones: {err}");
        }
    }

    fn emit(&self, event: PillEvent) {
        let handler = self.inner.handler.borrow().clone();
        if let Some(handler) = handler {
            handler(event);
        }
    }

    fn released(&self) {
        let command = self.inner.core.borrow_mut().pill.release();
        match command {
            Some(PillCommand::ToggleHandsFree) => self.emit(PillEvent::ToggleHandsFree),
            Some(PillCommand::EdgeChanged(edge)) => {
                let prefs = {
                    let mut core = self.inner.core.borrow_mut();
                    core.prefs.edge = edge;
                    core.prefs.clone()
                };
                self.save(&prefs);
            }
            None => {}
        }
    }

    fn save(&self, prefs: &Prefs) {
        if let Some(path) = &self.inner.prefs_path
            && let Err(err) = prefs.save(path)
        {
            eprintln!("could not save pill preferences: {err}");
        }
    }

    fn poll(&self) {
        let inner = &*self.inner;
        let window = inner.ui.window();
        let mut core = inner.core.borrow_mut();
        let core = &mut *core;
        let now = Instant::now();
        let dt = now - core.last_poll;
        core.last_poll = now;

        if !core.overlay_applied {
            let applied = window.with_winit_window(overlay::apply);
            if let Some(result) = applied {
                core.overlay_applied = true;
                if let Err(err) = result {
                    eprintln!("the pill is not a proper overlay: {err}");
                }
            }
        }
        if now >= core.next_display_poll {
            core.next_display_poll = now + DISPLAY_POLL;
            core.displays = overlay::displays(window.scale_factor());
            window.with_winit_window(raise);
        }

        let frame = core.pill.tick(&Tick {
            now: now - core.started,
            cursor: overlay::cursor_position(),
            displays: &core.displays,
            escape: overlay::escape_pressed(),
        });

        if let Some(origin) = frame.move_to {
            window.set_position(desktop_position(origin));
        }
        if core.hit_test != Some(frame.hit_test) {
            let applied = window.with_winit_window(|window| {
                let _ = window.set_cursor_hittest(frame.hit_test);
                raise(window);
            });
            core.hit_test = applied.map(|()| frame.hit_test);
        }

        let ui = &inner.ui;
        ui.set_look(look(&frame.shown));
        ui.set_capsule_x(frame.capsule.x as f32);
        ui.set_capsule_y(frame.capsule.y as f32);
        ui.set_capsule_width(frame.capsule.width as f32);
        ui.set_capsule_height(frame.capsule.height as f32);
        if core.card_until.is_some_and(|until| now >= until) {
            core.card_until = None;
            core.pill.set_card(false);
        }
        ui.set_card_shown(frame.card.is_some());
        if let Some(card) = frame.card {
            ui.set_card_x(card.x as f32);
            ui.set_card_y(card.y as f32);
            ui.set_card_width(card.width as f32);
            ui.set_card_height(card.height as f32);
        }
        if core.toast.as_ref().is_some_and(|(_, until)| now >= *until) {
            core.toast = None;
        }
        let toast = core.toast.as_ref().map(|(text, _)| text.as_str());
        ui.set_tooltip(toast.or(frame.shown.tooltip()).unwrap_or_default().into());
        let bar_scale = frame.shown.bar_scale();
        if let Some(scale) = bar_scale {
            ui.set_bar_scale(scale);
            for (i, height) in core.waveform.advance(dt).into_iter().enumerate() {
                inner.bars.set_row_data(i, height);
            }
        }

        let poll = match frame.shown {
            PillState::Hidden if frame.card.is_none() => HIDDEN_POLL,
            _ if bar_scale.is_some() || frame.hit_test => FRAME_POLL,
            _ => CURSOR_POLL,
        };
        if poll != core.poll {
            core.poll = poll;
            inner.timer.set_interval(poll);
        }
    }
}

fn raise(window: &WinitWindow) {
    if let Err(err) = overlay::raise(window) {
        eprintln!("could not raise the pill: {err}");
    }
}

/// Desktop units are points on macOS and pixels elsewhere (see `wf_core::pill::dock`).
fn desktop_position(origin: Point) -> WindowPosition {
    if cfg!(target_os = "macos") {
        WindowPosition::Logical(LogicalPosition::new(origin.x as f32, origin.y as f32))
    } else {
        WindowPosition::Physical(PhysicalPosition::new(origin.x as i32, origin.y as i32))
    }
}

fn look(state: &PillState) -> PillLook {
    match state {
        PillState::Hidden => PillLook::Hidden,
        PillState::Resting => PillLook::Resting,
        PillState::Ready => PillLook::Ready,
        PillState::ListeningPtt => PillLook::Listening,
        PillState::ListeningHandsFree => PillLook::Locked,
        PillState::Processing => PillLook::Processing,
        PillState::Error { .. } => PillLook::Error,
        PillState::Meeting => PillLook::Meeting,
    }
}

/// One line of a transcript, short enough for a menu.
fn menu_label(text: &str) -> SharedString {
    let line = text.lines().next().unwrap_or_default();
    match line.char_indices().nth(MENU_LABEL_CHARS) {
        Some((end, _)) => format!("{}…", &line[..end]).into(),
        None => line.into(),
    }
}

fn string_model<'a>(items: impl IntoIterator<Item = &'a str>) -> ModelRc<SharedString> {
    let items: Vec<SharedString> = items.into_iter().map(SharedString::from).collect();
    ModelRc::from(Rc::new(VecModel::from(items)))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use i_slint_backend_testing::{ElementHandle, init_integration_test_with_system_time};
    use slint::platform::{PointerEventButton, WindowEvent};
    use slint::{ComponentHandle, LogicalPosition, ModelRc, VecModel};

    use crate::{PillLook, PillWindow};

    /// A window showing `look`, with a log of what fired.
    fn pill_window(look: PillLook) -> (PillWindow, Rc<RefCell<Vec<&'static str>>>) {
        let ui = PillWindow::new().unwrap();
        ui.set_look(look);
        ui.set_capsule_x(82.0);
        ui.set_capsule_y(80.0);
        ui.set_capsule_width(148.0);
        ui.set_capsule_height(32.0);
        ui.set_bars(ModelRc::from(Rc::new(VecModel::from(vec![0.5_f32; 10]))));
        ui.set_card_shown(true);
        ui.set_card_x(42.0);
        ui.set_card_y(8.0);
        ui.set_card_width(236.0);
        ui.set_card_height(64.0);

        let fired = Rc::new(RefCell::new(Vec::new()));
        let log = |name: &'static str| {
            let fired = fired.clone();
            move || fired.borrow_mut().push(name)
        };
        ui.on_pill_pressed(log("pressed"));
        ui.on_pill_released(log("released"));
        ui.on_cancel_clicked(log("cancel"));
        ui.on_stop_clicked(log("stop"));
        ui.on_take_notes(log("take notes"));
        ui.on_decline_notes(log("not now"));
        (ui, fired)
    }

    fn element(ui: &PillWindow, id: &str) -> ElementHandle {
        ElementHandle::find_by_element_id(ui, id)
            .next()
            .unwrap_or_else(|| panic!("no element {id}"))
    }

    /// Clicks the element named `id` in a window showing `look`, and returns what fired.
    fn click(look: PillLook, id: &str) -> Vec<&'static str> {
        let (ui, fired) = pill_window(look);
        let element = element(&ui, id);
        slint::spawn_local(async move {
            element.single_click(PointerEventButton::Left).await;
            slint::quit_event_loop().unwrap();
        })
        .unwrap();
        slint::run_event_loop().unwrap();
        fired.borrow().clone()
    }

    /// Presses the right button on the capsule without releasing it. On x86_64 the menu is gone
    /// after `single_click`, for a reason not traced; a plain press and release keep it open.
    fn right_press(look: PillLook) -> (PillWindow, Vec<&'static str>) {
        let (ui, fired) = pill_window(look);
        let capsule = element(&ui, "PillWindow::capsule-touch");
        let size = capsule.size();
        let corner = capsule.absolute_position();
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position: LogicalPosition::new(
                corner.x + size.width / 2.0,
                corner.y + size.height / 2.0,
            ),
            button: PointerEventButton::Right,
        });
        let fired = fired.borrow().clone();
        (ui, fired)
    }

    // One test: Slint's test platform can only be installed once per process.
    #[test]
    fn clicks_reach_the_capsule_and_each_hands_free_button_on_its_own() {
        init_integration_test_with_system_time();
        assert_eq!(
            click(PillLook::Ready, "PillWindow::capsule-touch"),
            ["pressed", "released"]
        );
        assert_eq!(
            click(PillLook::Locked, "PillWindow::cancel-button"),
            ["cancel"]
        );
        assert_eq!(click(PillLook::Locked, "PillWindow::stop-button"), ["stop"]);
        assert_eq!(
            click(PillLook::Resting, "PillWindow::take-notes-button"),
            ["take notes"]
        );
        assert_eq!(
            click(PillLook::Resting, "PillWindow::decline-notes-button"),
            ["not now"]
        );

        let is_open = |look| {
            let (ui, fired) = right_press(look);
            assert_eq!(fired, Vec::<&str>::new(), "a right-click is not a press");
            ElementHandle::find_by_accessible_label(&ui, "Hide for 1 hour").count() > 0
        };
        assert!(
            is_open(PillLook::Ready),
            "right-click opens the menu when ready"
        );
        assert!(!is_open(PillLook::Processing), "the menu is only for Ready");
    }
}
