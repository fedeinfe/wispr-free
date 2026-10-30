//! Linux paste keystroke: XTest on X11; wtype, then ydotool, on Wayland.

use std::env;
use std::process::{Command, Stdio};

use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xproto::{ConnectionExt as _, KEY_PRESS_EVENT, KEY_RELEASE_EVENT, Keycode};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::wrapper::ConnectionExt as _;

use super::wayland::{wtype_args, wtype_enter_args, ydotool_args, ydotool_enter_args};
use super::{InsertError, PasteChord, PasteKeys};

const KEYSYM_V: u32 = 0x76;
const KEYSYM_INSERT: u32 = 0xff63;
const KEYSYM_RETURN: u32 = 0xff0d;
const KEYSYM_SHIFT_L: u32 = 0xffe1;
const KEYSYM_CONTROL_L: u32 = 0xffe3;

pub struct SystemPasteKeys;

impl PasteKeys for SystemPasteKeys {
    fn send(&mut self, chord: PasteChord) -> Result<(), InsertError> {
        if env::var_os("WAYLAND_DISPLAY").is_some() {
            send_on_wayland(&wtype_args(chord), &ydotool_args(chord))
        } else {
            let (modifier, key) = match chord {
                PasteChord::ShiftInsert => (KEYSYM_SHIFT_L, KEYSYM_INSERT),
                PasteChord::CtrlV | PasteChord::CmdV => (KEYSYM_CONTROL_L, KEYSYM_V),
            };
            send_with_xtest(&[modifier, key])
                .map_err(|error| InsertError::Keystroke(error.to_string()))
        }
    }

    fn send_enter(&mut self) -> Result<(), InsertError> {
        if env::var_os("WAYLAND_DISPLAY").is_some() {
            send_on_wayland(&wtype_enter_args(), &ydotool_enter_args())
        } else {
            send_with_xtest(&[KEYSYM_RETURN])
                .map_err(|error| InsertError::Keystroke(error.to_string()))
        }
    }
}

fn send_on_wayland(wtype: &[&str], ydotool: &[&str]) -> Result<(), InsertError> {
    // wtype needs the virtual-keyboard protocol (wlroots, KWin); ydotool works on GNOME too,
    // given its daemon.
    if run("wtype", wtype) || run("ydotool", ydotool) {
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

/// Presses `keysyms` in order and releases them in reverse.
fn send_with_xtest(keysyms: &[u32]) -> Result<(), Box<dyn std::error::Error>> {
    let (connection, screen) = x11rb::connect(None)?;
    if connection
        .extension_information(x11rb::protocol::xtest::X11_EXTENSION_NAME)?
        .is_none()
    {
        return Err("the X server has no XTEST extension".into());
    }
    let root = connection.setup().roots[screen].root;
    let keycodes = keysyms
        .iter()
        .map(|&keysym| keycode_for(&connection, keysym))
        .collect::<Result<Vec<_>, _>>()?;
    let presses = keycodes.iter().map(|&keycode| (KEY_PRESS_EVENT, keycode));
    let releases = keycodes
        .iter()
        .rev()
        .map(|&keycode| (KEY_RELEASE_EVENT, keycode));
    for (event, keycode) in presses.chain(releases) {
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
