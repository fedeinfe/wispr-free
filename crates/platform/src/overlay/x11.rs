//! X11 only. The window is created override-redirect (see the app's window attributes hook), so
//! no window manager keeps it on top and `raise` has to. Wayland has no global cursor and winit
//! has no layer-shell, so on Wayland these calls report nothing and the pill runs through XWayland
//! whenever `DISPLAY` is set.

use std::cell::OnceCell;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use wf_core::pill::dock::{Display, Point, Rect};
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{
    AtomEnum, ConfigureWindowAux, ConnectionExt as _, StackMode, Window,
};
use x11rb::rust_connection::RustConnection;

use super::OverlayError;

/// Esc on every keyboard map that follows the evdev or standard PC keycodes.
const ESCAPE_KEYCODE: usize = 9;

struct X11 {
    connection: RustConnection,
    root: Window,
}

thread_local! {
    static X11_STATE: OnceCell<Option<X11>> = const { OnceCell::new() };
}

fn with_x11<T>(f: impl FnOnce(&X11) -> Option<T>) -> Option<T> {
    X11_STATE.with(|state| {
        state
            .get_or_init(|| {
                let (connection, screen) = RustConnection::connect(None).ok()?;
                let root = connection.setup().roots.get(screen)?.root;
                Some(X11 { connection, root })
            })
            .as_ref()
            .and_then(f)
    })
}

pub fn apply(window: &impl HasWindowHandle) -> Result<(), OverlayError> {
    raise(window)
}

pub fn raise(window: &impl HasWindowHandle) -> Result<(), OverlayError> {
    let window = match window.window_handle()?.as_raw() {
        RawWindowHandle::Xlib(handle) => handle.window as Window,
        RawWindowHandle::Xcb(handle) => handle.window.get(),
        _ => return Ok(()),
    };
    with_x11(|x11| {
        let above = ConfigureWindowAux::new().stack_mode(StackMode::ABOVE);
        x11.connection.configure_window(window, &above).ok()?;
        x11.connection.flush().ok()
    });
    Ok(())
}

pub fn displays(window_scale: f32) -> Vec<Display> {
    with_x11(|x11| {
        let reply = x11
            .connection
            .randr_get_monitors(x11.root, true)
            .ok()?
            .reply()
            .ok()?;
        let work_area = desktop_work_area(x11);
        let displays = reply.monitors.iter().map(|monitor| {
            let bounds = Rect {
                x: f64::from(monitor.x),
                y: f64::from(monitor.y),
                width: f64::from(monitor.width),
                height: f64::from(monitor.height),
            };
            Display {
                bounds,
                work_area: work_area.map_or(bounds, |area| intersect(bounds, area)),
                units_per_point: f64::from(window_scale),
            }
        });
        Some(displays.collect())
    })
    .unwrap_or_default()
}

pub fn cursor_position() -> Option<Point> {
    with_x11(|x11| {
        let reply = x11.connection.query_pointer(x11.root).ok()?.reply().ok()?;
        Some(Point {
            x: f64::from(reply.root_x),
            y: f64::from(reply.root_y),
        })
    })
}

pub fn escape_pressed() -> bool {
    with_x11(|x11| {
        let keys = x11.connection.query_keymap().ok()?.reply().ok()?.keys;
        Some(keys[ESCAPE_KEYCODE / 8] & (1 << (ESCAPE_KEYCODE % 8)) != 0)
    })
    .unwrap_or(false)
}

/// `_NET_WORKAREA` of the first desktop. The window manager reports one rectangle per desktop,
/// not per monitor, so panels on secondary monitors are only approximated.
fn desktop_work_area(x11: &X11) -> Option<Rect> {
    let atom = x11
        .connection
        .intern_atom(true, b"_NET_WORKAREA")
        .ok()?
        .reply()
        .ok()?
        .atom;
    let reply = x11
        .connection
        .get_property(false, x11.root, atom, AtomEnum::CARDINAL, 0, 4)
        .ok()?
        .reply()
        .ok()?;
    let mut values = reply.value32()?;
    Some(Rect {
        x: f64::from(values.next()?),
        y: f64::from(values.next()?),
        width: f64::from(values.next()?),
        height: f64::from(values.next()?),
    })
}

fn intersect(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    Rect {
        x,
        y,
        width: ((a.x + a.width).min(b.x + b.width) - x).max(0.0),
        height: ((a.y + a.height).min(b.y + b.height) - y).max(0.0),
    }
}
