//! Linux paste keystroke: XTest on X11; wtype, then ydotool, on Wayland.

use std::env;
use std::process::{Command, Stdio};

use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xproto::{ConnectionExt as _, KEY_PRESS_EVENT, KEY_RELEASE_EVENT, Keycode};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::wrapper::ConnectionExt as _;

use super::wayland::{wtype_args, ydotool_args};
use super::{InsertError, PasteChord, PasteKeys};

const KEYSYM_V: u32 = 0x76;
const KEYSYM_INSERT: u32 = 0xff63;
const KEYSYM_SHIFT_L: u32 = 0xffe1;
const KEYSYM_CONTROL_L: u32 = 0xffe3;

pub struct SystemPasteKeys;

impl PasteKeys for SystemPasteKeys {
    fn send(&mut self, chord: PasteChord) -> Result<(), InsertError> {
        if env::var_os("WAYLAND_DISPLAY").is_some() {
            send_on_wayland(chord)
        } else {
            send_with_xtest(chord).map_err(|error| InsertError::Keystroke(error.to_string()))
        }
    }
}

fn send_on_wayland(chord: PasteChord) -> Result<(), InsertError> {
    // wtype needs the virtual-keyboard protocol (wlroots, KWin); ydotool works on GNOME too,
    // given its daemon.
    if run("wtype", &wtype_args(chord)) || run("ydotool", &ydotool_args(chord)) {
        Ok(())
    } else {
        Err(InsertError::Keystroke(
            "neither wtype nor ydotool could send the keystroke".into(),
        ))
    }
}

fn run(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn send_with_xtest(chord: PasteChord) -> Result<(), Box<dyn std::error::Error>> {
    let (connection, screen) = x11rb::connect(None)?;
    if connection
        .extension_information(x11rb::protocol::xtest::X11_EXTENSION_NAME)?
        .is_none()
    {
        return Err("the X server has no XTEST extension".into());
    }
    let root = connection.setup().roots[screen].root;
    let (modifier, key) = match chord {
        PasteChord::ShiftInsert => (KEYSYM_SHIFT_L, KEYSYM_INSERT),
        PasteChord::CtrlV | PasteChord::CmdV => (KEYSYM_CONTROL_L, KEYSYM_V),
    };
    let modifier = keycode_for(&connection, modifier)?;
    let key = keycode_for(&connection, key)?;
    for (event, keycode) in [
        (KEY_PRESS_EVENT, modifier),
        (KEY_PRESS_EVENT, key),
        (KEY_RELEASE_EVENT, key),
        (KEY_RELEASE_EVENT, modifier),
    ] {
        connection.xtest_fake_input(event, keycode, 0, root, 0, 0, 0)?;
    }
    connection.sync()?;
    Ok(())
}

/// The keycode that produces `keysym` in the current layout.
fn keycode_for(
    connection: &impl Connection,
    keysym: u32,
) -> Result<Keycode, Box<dyn std::error::Error>> {
    let setup = connection.setup();
    let first = setup.min_keycode;
    let count = setup.max_keycode - first + 1;
    let mapping = connection.get_keyboard_mapping(first, count)?.reply()?;
    let per_keycode = usize::from(mapping.keysyms_per_keycode);
    mapping
        .keysyms
        .chunks(per_keycode)
        .position(|keysyms| keysyms.contains(&keysym))
        .map(|index| first + index as u8)
        .ok_or_else(|| format!("no keycode for keysym {keysym:#x}").into())
}
