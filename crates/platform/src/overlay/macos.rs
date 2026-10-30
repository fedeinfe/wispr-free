//! winit only creates `NSWindow`s, and only an `NSPanel` can be non-activating, so the window's
//! class is swapped for a panel subclass once it exists.

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObjectProtocol};
use objc2::{ClassType, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSEvent, NSPanel, NSScreen, NSView, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_graphics::{CGEventSource, CGEventSourceStateID};
use objc2_foundation::NSRect;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use wf_core::pill::dock::{Display, Point, Rect};

use super::OverlayError;

/// `NSScreenSaverWindowLevel`, which also sits over full-screen apps.
const SCREEN_SAVER_LEVEL: isize = 1000;
/// `kVK_Escape`.
const ESCAPE_KEYCODE: u16 = 53;

define_class!(
    #[unsafe(super(NSPanel))]
    #[thread_kind = MainThreadOnly]
    #[name = "WfOverlayPanel"]
    struct OverlayPanel;

    impl OverlayPanel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            false
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool {
            false
        }
    }
);

pub fn apply(window: &impl HasWindowHandle) -> Result<(), OverlayError> {
    let window = ns_window(window)?;
    become_panel(&window)?;
    let panel = window
        .downcast_ref::<NSPanel>()
        .ok_or(OverlayError::UnsupportedWindow)?;

    panel.setStyleMask(panel.styleMask() | NSWindowStyleMask::NonactivatingPanel);
    prevent_activation(panel)?;
    panel.setFloatingPanel(true);
    panel.setBecomesKeyOnlyIfNeeded(true);
    panel.setWorksWhenModal(true);
    // Panels hide when their app deactivates by default, and this app is never active.
    panel.setHidesOnDeactivate(false);
    panel.setLevel(SCREEN_SAVER_LEVEL);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    panel.setHasShadow(false);
    Ok(())
}

/// AppKit only tells the window server that a panel is non-activating during panel init, so
/// setting the style mask on our existing window changes nothing: a click still activates the
/// app. This private call is what init uses. If a macOS update drops it, `apply` reports it.
fn prevent_activation(panel: &NSPanel) -> Result<(), OverlayError> {
    let selector = sel!(_setPreventsActivation:);
    if !panel.respondsToSelector(selector) {
        return Err(OverlayError::Os("no _setPreventsActivation:".into()));
    }
    // SAFETY: checked above that the panel responds to it. It takes one BOOL and returns void.
    let () = unsafe { msg_send![panel, _setPreventsActivation: true] };
    Ok(())
}

pub fn raise(_window: &impl HasWindowHandle) -> Result<(), OverlayError> {
    Ok(())
}

pub fn displays(_window_scale: f32) -> Vec<Display> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Vec::new();
    };
    let screens = NSScreen::screens(mtm);
    let Some(primary_height) = screens.firstObject().map(|s| s.frame().size.height) else {
        return Vec::new();
    };
    screens
        .iter()
        .map(|screen| Display {
            bounds: flipped(screen.frame(), primary_height),
            work_area: flipped(screen.visibleFrame(), primary_height),
            units_per_point: 1.0,
        })
        .collect()
}

pub fn cursor_position() -> Option<Point> {
    let mtm = MainThreadMarker::new()?;
    let primary_height = NSScreen::screens(mtm).firstObject()?.frame().size.height;
    let location = NSEvent::mouseLocation();
    Some(Point {
        x: location.x,
        y: primary_height - location.y,
    })
}

pub fn escape_pressed() -> bool {
    CGEventSource::key_state(CGEventSourceStateID::CombinedSessionState, ESCAPE_KEYCODE)
}

/// AppKit's global coordinates start at the primary screen's bottom-left corner.
fn flipped(frame: NSRect, primary_height: f64) -> Rect {
    Rect {
        x: frame.origin.x,
        y: primary_height - frame.origin.y - frame.size.height,
        width: frame.size.width,
        height: frame.size.height,
    }
}

fn ns_window(window: &impl HasWindowHandle) -> Result<Retained<NSWindow>, OverlayError> {
    let RawWindowHandle::AppKit(handle) = window.window_handle()?.as_raw() else {
        return Err(OverlayError::UnsupportedWindow);
    };
    if MainThreadMarker::new().is_none() {
        return Err(OverlayError::NotOnUiThread);
    }
    // SAFETY: a valid AppKit handle points to a live NSView, and we are on the main thread.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    view.window().ok_or(OverlayError::UnsupportedWindow)
}

fn become_panel(window: &NSWindow) -> Result<(), OverlayError> {
    let panel_class: &AnyClass = OverlayPanel::class();
    if window.class().instance_size() != panel_class.instance_size() {
        return Err(OverlayError::Os(format!(
            "panel layout differs from {} ({} vs {} bytes)",
            window.class().name().to_string_lossy(),
            panel_class.instance_size(),
            window.class().instance_size(),
        )));
    }
    let object: &AnyObject = window.as_ref();
    // SAFETY: OverlayPanel is an NSWindow subclass of the same instance size (checked above) that
    // adds only method overrides, and winit reaches the window through NSWindow methods alone.
    unsafe { AnyObject::set_class(object, panel_class) };
    Ok(())
}
