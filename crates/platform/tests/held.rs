//! The backend's own record of held keys, which repairs releases the OS never delivered.

use wf_core::shortcut::Side::Left;
use wf_core::shortcut::{Key, KeyEvent, MouseButton};
use wf_platform::keys::held::Held;

const FORWARD: Key = Key::Mouse(MouseButton::Forward);

#[test]
fn a_press_reports_earlier_keys_the_os_says_are_up_as_released() {
    let mut held = Held::default();
    let all_down = |_| Some(true);
    assert!(
        held.on_event(KeyEvent::press(Key::Ctrl(Left)), all_down)
            .is_empty()
    );
    assert!(
        held.on_event(KeyEvent::press(Key::Meta(Left)), all_down)
            .is_empty()
    );
    assert!(
        held.on_event(KeyEvent::press(Key::Char('Q')), all_down)
            .is_empty()
    );
    // The screen locked: none of the three releases arrived.

    let only_forward = |key| Some(key == FORWARD);
    let missed = held.on_event(KeyEvent::press(FORWARD), only_forward);

    assert_eq!(
        missed,
        [
            KeyEvent::release(Key::Ctrl(Left)),
            KeyEvent::release(Key::Meta(Left)),
            KeyEvent::release(Key::Char('Q')),
        ]
    );
    assert_eq!(
        held.on_event(KeyEvent::press(Key::Space), |_| Some(false)),
        [KeyEvent::release(FORWARD)]
    );
}

#[test]
fn keys_still_down_or_of_unknown_state_are_kept() {
    let mut held = Held::default();
    held.on_event(KeyEvent::press(Key::Fn), |_| Some(true));
    held.on_event(KeyEvent::press(Key::Alt(Left)), |_| Some(true));

    let missed = held.on_event(KeyEvent::press(Key::Space), |key| match key {
        Key::Fn => None,
        _ => Some(true),
    });

    assert!(missed.is_empty());
}

#[test]
fn a_delivered_release_forgets_the_key() {
    let mut held = Held::default();
    held.on_event(KeyEvent::press(FORWARD), |_| Some(true));
    held.on_event(KeyEvent::release(FORWARD), |_| Some(false));

    assert!(
        held.on_event(KeyEvent::press(FORWARD), |_| Some(false))
            .is_empty()
    );
}

#[test]
fn a_repeated_press_does_not_ask_about_itself() {
    let mut held = Held::default();
    held.on_event(KeyEvent::press(Key::Char('A')), |_| Some(true));

    assert!(
        held.on_event(KeyEvent::press(Key::Char('A')), |_| Some(false))
            .is_empty()
    );
}
