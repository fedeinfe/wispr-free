//! Native key codes to our layout-independent keys. Expected values are taken from the OS headers:
//! HIToolbox Events.h (macOS), WinUser.h (Windows) and linux/input-event-codes.h (Linux).

use wf_core::shortcut::Side::{Left, Right};
use wf_core::shortcut::{Key, MouseButton};
use wf_platform::keys::keymap::{linux_key, mac_key, windows_key};

#[test]
fn macos_virtual_keycodes() {
    let cases = [
        (0x00, Key::Char('A')),
        (0x09, Key::Char('V')),
        (0x12, Key::Char('1')),
        (0x1D, Key::Char('0')),
        (0x32, Key::Char('`')),
        (0x24, Key::Enter),
        (0x30, Key::Tab),
        (0x31, Key::Space),
        (0x33, Key::Backspace),
        (0x35, Key::Esc),
        (0x36, Key::Meta(Right)),
        (0x37, Key::Meta(Left)),
        (0x38, Key::Shift(Left)),
        (0x39, Key::CapsLock),
        (0x3A, Key::Alt(Left)),
        (0x3B, Key::Ctrl(Left)),
        (0x3C, Key::Shift(Right)),
        (0x3D, Key::Alt(Right)),
        (0x3E, Key::Ctrl(Right)),
        (0x3F, Key::Fn),
        (0x5A, Key::F(20)),
        (0x72, Key::Insert),
        (0x75, Key::Delete),
        (0x7A, Key::F(1)),
        (0x7B, Key::ArrowLeft),
        (0x7E, Key::ArrowUp),
        (0x7F, Key::Other(0x7F)),
    ];
    for (code, key) in cases {
        assert_eq!(mac_key(code), key, "{code:#x}");
    }
}

#[test]
fn windows_virtual_keys() {
    let cases = [
        (0x41, Key::Char('A')),
        (0x5A, Key::Char('Z')),
        (0x30, Key::Char('0')),
        (0xC0, Key::Char('`')),
        (0xBA, Key::Char(';')),
        (0x0D, Key::Enter),
        (0x1B, Key::Esc),
        (0x20, Key::Space),
        (0x2C, Key::PrintScreen),
        (0x2E, Key::Delete),
        (0x5B, Key::Meta(Left)),
        (0x5C, Key::Meta(Right)),
        (0xA0, Key::Shift(Left)),
        (0xA1, Key::Shift(Right)),
        (0xA2, Key::Ctrl(Left)),
        (0xA3, Key::Ctrl(Right)),
        (0xA4, Key::Alt(Left)),
        (0xA5, Key::Alt(Right)),
        (0x70, Key::F(1)),
        (0x87, Key::F(24)),
        (0xFF, Key::Other(0xFF)),
    ];
    for (code, key) in cases {
        assert_eq!(windows_key(code), key, "{code:#x}");
    }
}

#[test]
fn linux_evdev_codes() {
    let cases = [
        (1, Key::Esc),
        (2, Key::Char('1')),
        (11, Key::Char('0')),
        (14, Key::Backspace),
        (15, Key::Tab),
        (16, Key::Char('Q')),
        (28, Key::Enter),
        (29, Key::Ctrl(Left)),
        (30, Key::Char('A')),
        (41, Key::Char('`')),
        (42, Key::Shift(Left)),
        (47, Key::Char('V')),
        (54, Key::Shift(Right)),
        (56, Key::Alt(Left)),
        (57, Key::Space),
        (59, Key::F(1)),
        (87, Key::F(11)),
        (88, Key::F(12)),
        (97, Key::Ctrl(Right)),
        (99, Key::PrintScreen),
        (100, Key::Alt(Right)),
        (103, Key::ArrowUp),
        (111, Key::Delete),
        (125, Key::Meta(Left)),
        (126, Key::Meta(Right)),
        (183, Key::F(13)),
        (194, Key::F(24)),
        (0x112, Key::Mouse(MouseButton::Middle)),
        (0x113, Key::Mouse(MouseButton::Back)),
        (0x114, Key::Mouse(MouseButton::Forward)),
        (0x2FF, Key::Other(0x2FF)),
    ];
    for (code, key) in cases {
        assert_eq!(linux_key(code), key, "{code:#x}");
    }
}

#[test]
fn macos_flags_changed_tells_press_from_release_per_side() {
    use wf_platform::keys::keymap::mac_modifier_pressed;
    let cases = [
        ("left Cmd down", 0x37, 0x08, Some(true)),
        (
            "left Cmd up with right Cmd still down",
            0x37,
            0x10,
            Some(false),
        ),
        ("right Ctrl down", 0x3E, 0x2000, Some(true)),
        ("Fn down", 0x3F, 0x80_0000, Some(true)),
        ("Fn up", 0x3F, 0, Some(false)),
        ("Caps Lock has no held state", 0x39, 0x1_0000, None),
        ("not a modifier", 0x00, 0x08, None),
    ];
    for (name, code, flags, expected) in cases {
        assert_eq!(mac_modifier_pressed(code, flags), expected, "{name}");
    }
}

#[test]
fn macos_live_state_inputs_invert_the_keycodes_and_buttons() {
    use wf_platform::keys::keymap::{MacInput, mac_input};

    for code in 0..=0x7F {
        let key = mac_key(code);
        if key != Key::Fn {
            assert_eq!(mac_input(key), Some(MacInput::Key(code)), "{code:#x}");
        }
    }
    assert_eq!(mac_input(Key::Fn), None);
    assert_eq!(
        mac_input(Key::Mouse(MouseButton::Middle)),
        Some(MacInput::Button(2))
    );
    assert_eq!(
        mac_input(Key::Mouse(MouseButton::Back)),
        Some(MacInput::Button(3))
    );
    assert_eq!(
        mac_input(Key::Mouse(MouseButton::Forward)),
        Some(MacInput::Button(4))
    );
    assert_eq!(
        mac_input(Key::Mouse(MouseButton::Other(5))),
        Some(MacInput::Button(5))
    );
    assert_eq!(mac_input(Key::PrintScreen), None);
}
