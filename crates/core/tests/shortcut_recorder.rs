//! The shortcut recorder's capture loop (spec §2): the combo is committed on release, Esc alone
//! aborts, and reset to default is available.

use wf_core::shortcut::Key::{Alt, Char, Ctrl, Esc, Fn, Meta, Mouse, Shift, Space};
use wf_core::shortcut::Platform::{MacOs, Windows};
use wf_core::shortcut::Side::{Either, Left, Right};
use wf_core::shortcut::recorder::{RecorderStep, ShortcutRecorder, reset_to_default};
use wf_core::shortcut::{Action, Bindings, Combo, Key, KeyEvent, MouseButton, ShortcutError};

fn recorder(action: Action) -> ShortcutRecorder {
    ShortcutRecorder::new(action, Bindings::default_for(MacOs), MacOs)
}

fn press(recorder: &mut ShortcutRecorder, keys: &[Key]) -> Vec<RecorderStep> {
    keys.iter()
        .map(|&key| recorder.on_key(KeyEvent::press(key)))
        .collect()
}

fn release(recorder: &mut ShortcutRecorder, keys: &[Key]) -> Vec<RecorderStep> {
    keys.iter()
        .map(|&key| recorder.on_key(KeyEvent::release(key)))
        .collect()
}

/// Presses `keys`, releases them in the same order, and returns the final step.
fn tap(recorder: &mut ShortcutRecorder, keys: &[Key]) -> RecorderStep {
    press(recorder, keys);
    release(recorder, keys).pop().unwrap()
}

#[test]
fn nothing_is_committed_until_every_key_is_released() {
    let mut recorder = recorder(Action::PasteLast);
    let keys = [Ctrl(Left), Alt(Left), Char('J')];

    assert_eq!(
        press(&mut recorder, &keys).pop().unwrap(),
        RecorderStep::Recording(Combo::new(keys)),
    );
    assert_eq!(
        release(&mut recorder, &[Char('J')]),
        [RecorderStep::Recording(Combo::new([Ctrl(Left), Alt(Left)]))],
    );
    assert_eq!(
        release(&mut recorder, &[Alt(Left)]),
        [RecorderStep::Recording(Combo::new([Ctrl(Left)]))],
    );
    assert!(matches!(
        release(&mut recorder, &[Ctrl(Left)]).pop().unwrap(),
        RecorderStep::Committed(_)
    ));
}

#[test]
fn the_combo_is_the_largest_set_held_together() {
    let mut recorder = recorder(Action::PasteLast);

    press(&mut recorder, &[Ctrl(Left), Char('K')]);
    release(&mut recorder, &[Ctrl(Left)]);
    let step = release(&mut recorder, &[Char('K')]).pop().unwrap();

    assert_eq!(
        step,
        RecorderStep::Committed(Combo::new([Ctrl(Either), Char('K')]))
    );
}

#[test]
fn a_combo_with_a_regular_key_works_with_either_side_of_its_modifiers() {
    let mut recorder = recorder(Action::PasteLast);
    assert_eq!(
        tap(&mut recorder, &[Shift(Right), Ctrl(Left), Char('J')]),
        RecorderStep::Committed(Combo::new([Ctrl(Either), Shift(Either), Char('J')])),
    );
}

#[test]
fn a_combo_of_modifiers_only_keeps_its_sides() {
    let mut recorder = recorder(Action::PushToTalk);
    assert_eq!(
        tap(&mut recorder, &[Meta(Right)]),
        RecorderStep::Committed(Combo::new([Meta(Right)])),
    );
}

#[test]
fn modifiers_come_first_in_a_fixed_order_whatever_the_press_order() {
    let mut recorder = recorder(Action::PasteLast);
    assert_eq!(
        tap(&mut recorder, &[Char('J'), Alt(Left), Ctrl(Left)]),
        RecorderStep::Committed(Combo::new([Ctrl(Either), Alt(Either), Char('J')])),
    );
}

#[test]
fn esc_alone_aborts_as_soon_as_it_is_pressed() {
    let mut recorder = recorder(Action::PushToTalk);
    assert_eq!(press(&mut recorder, &[Esc]), [RecorderStep::Aborted]);
}

#[test]
fn esc_with_a_modifier_is_part_of_the_combo() {
    let mut recorder = recorder(Action::PushToTalk);
    assert_eq!(
        tap(&mut recorder, &[Shift(Left), Esc]),
        RecorderStep::Committed(Combo::new([Shift(Either), Esc])),
    );
}

#[test]
fn esc_can_be_recorded_for_cancel_since_it_is_the_one_action_allowing_it() {
    let mut recorder = ShortcutRecorder::new(
        Action::Cancel,
        Bindings {
            cancel: Combo::new([Ctrl(Either), Char('J')]),
            ..Bindings::default_for(MacOs)
        },
        MacOs,
    );
    assert_eq!(
        tap(&mut recorder, &[Esc]),
        RecorderStep::Committed(Combo::new([Esc]))
    );
}

#[test]
fn a_refused_combo_says_why_and_recording_continues() {
    let mut recorder = recorder(Action::PushToTalk);

    assert_eq!(
        tap(&mut recorder, &[Char('Q')]),
        RecorderStep::Rejected(Combo::new([Char('Q')]), ShortcutError::NeedsModifier),
    );
    assert_eq!(
        tap(&mut recorder, &[Meta(Left), Char('C')]),
        RecorderStep::Rejected(
            Combo::new([Meta(Either), Char('C')]),
            ShortcutError::Reserved
        ),
    );
    assert_eq!(
        tap(&mut recorder, &[Ctrl(Left), Ctrl(Right), Char('J')]),
        RecorderStep::Rejected(
            Combo::new([Ctrl(Either), Ctrl(Either), Char('J')]),
            ShortcutError::BothSidesOfModifier
        ),
    );
    assert_eq!(
        tap(&mut recorder, &[Ctrl(Left), Alt(Left), Shift(Left), Space]),
        RecorderStep::Rejected(
            Combo::new([Ctrl(Either), Shift(Either), Alt(Either), Space]),
            ShortcutError::TooManyKeys
        ),
    );
    assert_eq!(
        tap(&mut recorder, &[Ctrl(Left), Char('J')]),
        RecorderStep::Committed(Combo::new([Ctrl(Either), Char('J')])),
    );
}

#[test]
fn a_combo_taken_by_another_action_is_refused() {
    let mut recorder = recorder(Action::CopyLast);
    assert_eq!(
        tap(&mut recorder, &[Alt(Left), Char('M')]),
        RecorderStep::Rejected(
            Combo::new([Alt(Either), Char('M')]),
            ShortcutError::Conflict(Action::MeetingNotes)
        ),
    );
}

#[test]
fn the_release_of_the_key_that_opened_the_recorder_is_ignored() {
    let mut recorder = recorder(Action::PasteLast);
    assert_eq!(
        release(&mut recorder, &[Key::Enter]),
        [RecorderStep::Recording(Combo::new([]))],
    );
}

#[test]
fn key_repeat_does_not_add_a_key() {
    let mut recorder = recorder(Action::PasteLast);
    press(
        &mut recorder,
        &[Ctrl(Left), Char('J'), Char('J'), Char('J')],
    );
    assert_eq!(
        release(&mut recorder, &[Char('J'), Ctrl(Left)])
            .pop()
            .unwrap(),
        RecorderStep::Committed(Combo::new([Ctrl(Either), Char('J')])),
    );
}

#[test]
fn a_non_primary_mouse_button_can_stand_alone() {
    let mut recorder = recorder(Action::PushToTalk);
    let button = Mouse(MouseButton::Back);
    assert_eq!(
        tap(&mut recorder, &[button]),
        RecorderStep::Committed(Combo::new([button]))
    );
}

#[test]
fn fn_comes_before_the_other_modifiers() {
    let mut recorder = recorder(Action::PasteLast);
    assert_eq!(
        tap(&mut recorder, &[Space, Ctrl(Left), Fn]),
        RecorderStep::Committed(Combo::new([Fn, Ctrl(Either), Space])),
    );
}

#[test]
fn reset_restores_the_platform_default() {
    let mut bindings = Bindings::default_for(Windows);
    bindings.paste_last = Combo::new([Ctrl(Either), Char('J')]);

    assert_eq!(
        reset_to_default(Action::PasteLast, &mut bindings, Windows),
        Ok(())
    );
    assert_eq!(bindings, Bindings::default_for(Windows));
}

#[test]
fn reset_is_refused_while_another_action_holds_the_default() {
    let mut bindings = Bindings::default_for(MacOs);
    bindings.meeting_notes = bindings.paste_last.clone();
    bindings.paste_last = Combo::new([Ctrl(Either), Char('J')]);

    assert_eq!(
        reset_to_default(Action::PasteLast, &mut bindings, MacOs),
        Err(ShortcutError::Conflict(Action::MeetingNotes)),
    );
    assert_eq!(bindings.paste_last, Combo::new([Ctrl(Either), Char('J')]));
}

#[test]
fn set_replaces_the_binding_of_one_action() {
    let mut bindings = Bindings::default_for(MacOs);
    let combo = Combo::new([Ctrl(Either), Char('J')]);

    bindings.set(Action::CopyLast, combo.clone());

    assert_eq!(bindings.get(Action::CopyLast), &combo);
    assert_eq!(bindings.push_to_talk, Combo::new([Fn]));
}
