//! Linux backend, best effort (spec §7): the app and title of the active X11 window through
//! EWMH. Focus stays Unknown so the paste always happens. AT-SPI text reads are not done yet,
//! and native Wayland exposes no focused window at all, so `app` is `None` there.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, Window};
use x11rb::rust_connection::RustConnection;

use super::{AppIdentity, Context, ContextDepth, ContextSource, Focus};

/// Linux has no Accessibility permission to wait for.
pub fn accessibility_trusted() -> bool {
    true
}

pub struct SystemContext;

impl ContextSource for SystemContext {
    fn read(&self, _depth: ContextDepth) -> Context {
        read_x11().unwrap_or(Context {
            app: None,
            window_title: None,
            focus: Focus::Unknown,
            text: None,
        })
    }
}

fn read_x11() -> Option<Context> {
    let (connection, screen) = x11rb::connect(None).ok()?;
    let root = connection.setup().roots[screen].root;
    let active = property(
        &connection,
        root,
        "_NET_ACTIVE_WINDOW",
        AtomEnum::WINDOW.into(),
    )?;
    let window = u32::from_ne_bytes(active.get(..4)?.try_into().ok()?);
    if window == 0 {
        return Some(Context {
            app: None,
            window_title: None,
            focus: Focus::Nothing,
            text: None,
        });
    }
    Some(Context {
        app: app_of(&connection, window),
        window_title: title_of(&connection, window),
        focus: Focus::Unknown,
        text: None,
    })
}

fn app_of(connection: &RustConnection, window: Window) -> Option<AppIdentity> {
    // WM_CLASS holds "instance\0class\0".
    let class = property(connection, window, "WM_CLASS", AtomEnum::STRING.into())?;
    let mut parts = class
        .split(|&byte| byte == 0)
        .filter(|part| !part.is_empty());
    let instance = String::from_utf8_lossy(parts.next()?).into_owned();
    let name = parts
        .next()
        .map(|c| String::from_utf8_lossy(c).into_owned());
    let pid = property(connection, window, "_NET_WM_PID", AtomEnum::CARDINAL.into())
        .and_then(|bytes| Some(u32::from_ne_bytes(bytes.get(..4)?.try_into().ok()?)));
    Some(AppIdentity {
        id: instance,
        name,
        pid,
    })
}

fn title_of(connection: &RustConnection, window: Window) -> Option<String> {
    let utf8 = intern(connection, "UTF8_STRING")?;
    let title = property(connection, window, "_NET_WM_NAME", utf8)
        .or_else(|| property(connection, window, "WM_NAME", AtomEnum::STRING.into()))?;
    Some(String::from_utf8_lossy(&title).into_owned())
}

fn property(connection: &RustConnection, window: Window, name: &str, kind: u32) -> Option<Vec<u8>> {
    let atom = intern(connection, name)?;
    let reply = connection
        .get_property(false, window, atom, kind, 0, 1024)
        .ok()?
        .reply()
        .ok()?;
    (!reply.value.is_empty()).then_some(reply.value)
}

fn intern(connection: &RustConnection, name: &str) -> Option<u32> {
    let reply = connection.intern_atom(false, name.as_bytes()).ok()?;
    Some(reply.reply().ok()?.atom)
}
