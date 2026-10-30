//! Windows paste keystroke through SendInput.

use std::mem::size_of;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_CONTROL, VK_INSERT, VK_SHIFT, VK_V,
};

use super::{InsertError, PasteChord, PasteKeys};

pub struct SystemPasteKeys;

impl PasteKeys for SystemPasteKeys {
    fn send(&mut self, chord: PasteChord) -> Result<(), InsertError> {
        // Virtual keys follow the layout, so VK_V is the "V" key on any of them.
        let (modifier, key, extended) = match chord {
            PasteChord::ShiftInsert => (VK_SHIFT, VK_INSERT, KEYEVENTF_EXTENDEDKEY),
            PasteChord::CtrlV | PasteChord::CmdV => (VK_CONTROL, VK_V, KEYBD_EVENT_FLAGS(0)),
        };
        let inputs = [
            key_input(modifier, KEYBD_EVENT_FLAGS(0)),
            key_input(key, extended),
            key_input(key, extended | KEYEVENTF_KEYUP),
            key_input(modifier, KEYEVENTF_KEYUP),
        ];
        let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
        if sent as usize == inputs.len() {
            Ok(())
        } else {
            Err(InsertError::Keystroke(format!(
                "SendInput sent {sent} of {} events",
                inputs.len()
            )))
        }
    }
}

fn key_input(key: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}
