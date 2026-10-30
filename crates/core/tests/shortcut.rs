//! The shortcut recorder's rules (spec §2), which Settings reuses in ticket 12.

use wf_core::shortcut::Key::{Alt, Char, Ctrl, Esc, F, Fn, Meta, Mouse, Shift, Space, Tab};
use wf_core::shortcut::Platform::{Linux, MacOs, Windows};
use wf_core::shortcut::Side::{Either, Left, Right};
use wf_core::shortcut::{
    Action, Bindings, Combo, Key, MouseButton, Platform, ShortcutError, validate,
};

type Case<'a> = (&'a str, Action, &'a [Key], Result<(), ShortcutError>);

fn check(platform: Platform, action: Action, keys: &[Key]) -> Result<(), ShortcutError> {
    validate(
        action,
        &Combo::new(keys),
        &Bindings::default_for(platform),
        platform,
    )
}

#[test]
fn every_default_binding_is_valid_on_its_platform() {
    for platform in [MacOs, Windows, Linux] {
        let bindings = Bindings::default_for(platform);
        for action in Action::ALL {
            assert_eq!(
                validate(action, bindings.get(action), &bindings, platform),
                Ok(()),
                "{platform:?} {action:?}"
            );
        }
    }
}

#[test]
fn a_combo_needs_a_modifier_or_a_non_primary_mouse_button() {
    let cases: &[Case] = &[
        ("empty", Action::PushToTalk, &[], Err(ShortcutError::Empty)),
        (
            "a lone letter",
            Action::PushToTalk,
            &[Char('Q')],
            Err(ShortcutError::NeedsModifier),
        ),
        (
            "letter and F key",
            Action::PasteLast,
            &[F(5), Char('Q')],
            Err(ShortcutError::NeedsModifier),
        ),
        (
            "modifier and letter",
            Action::PasteLast,
            &[Ctrl(Either), Char('J')],
            Ok(()),
        ),
        (
            "a lone modifier",
            Action::PushToTalk,
            &[Meta(Right)],
            Ok(()),
        ),
        (
            "a side mouse button",
            Action::PushToTalk,
            &[Mouse(MouseButton::Back)],
            Ok(()),
        ),
        (
            "mouse button and letter",
            Action::PasteLast,
            &[Mouse(MouseButton::Middle), Char('Q')],
            Ok(()),
        ),
        ("Esc alone for Cancel", Action::Cancel, &[Esc], Ok(())),
        (
            "Esc alone for anything else",
            Action::PasteLast,
            &[Esc],
            Err(ShortcutError::NeedsModifier),
        ),
    ];
    for (name, action, keys, expected) in cases {
        assert_eq!(check(MacOs, *action, keys), *expected, "{name}");
    }
}

#[test]
fn a_combo_has_at_most_three_keys_and_never_both_sides_of_a_modifier() {
    let cases: &[(&str, &[Key], Result<(), ShortcutError>)] = &[
        ("three keys", &[Ctrl(Left), Alt(Left), Char('J')], Ok(())),
        (
            "four keys",
            &[Ctrl(Left), Alt(Left), Shift(Left), Char('J')],
            Err(ShortcutError::TooManyKeys),
        ),
        (
            "left and right",
            &[Ctrl(Left), Ctrl(Right)],
            Err(ShortcutError::BothSidesOfModifier),
        ),
        (
            "either and a side",
            &[Alt(Either), Alt(Right), Char('J')],
            Err(ShortcutError::BothSidesOfModifier),
        ),
        ("different modifiers", &[Ctrl(Left), Alt(Right)], Ok(())),
    ];
    for (name, keys, expected) in cases {
        assert_eq!(check(MacOs, Action::PushToTalk, keys), *expected, "{name}");
    }
}

#[test]
fn a_combo_is_unique_across_actions() {
    let cases: &[Case] = &[
        (
            "another action's combo",
            Action::CopyLast,
            &[Alt(Either), Char('M')],
            Err(ShortcutError::Conflict(Action::MeetingNotes)),
        ),
        (
            "a sided version of it",
            Action::CopyLast,
            &[Alt(Right), Char('M')],
            Err(ShortcutError::Conflict(Action::MeetingNotes)),
        ),
        (
            "in a different key order",
            Action::CopyLast,
            &[Char('M'), Alt(Either)],
            Err(ShortcutError::Conflict(Action::MeetingNotes)),
        ),
        (
            "its own combo",
            Action::MeetingNotes,
            &[Alt(Either), Char('M')],
            Ok(()),
        ),
        (
            "a superset of another combo",
            Action::CopyLast,
            &[Fn, Alt(Either), Char('M')],
            Ok(()),
        ),
        (
            "the other side than a sided binding",
            Action::PushToTalk,
            &[Fn, Ctrl(Right)],
            Ok(()),
        ),
    ];
    for (name, action, keys, expected) in cases {
        assert_eq!(check(MacOs, *action, keys), *expected, "{name}");
    }
}

#[test]
fn os_reserved_combos_are_refused() {
    let cases: &[(Platform, &[Key])] = &[
        (MacOs, &[Meta(Left), Char('C')]),
        (MacOs, &[Meta(Right), Char('V')]),
        (MacOs, &[Meta(Either), Char('Q')]),
        (MacOs, &[Meta(Left), Tab]),
        (MacOs, &[Meta(Left), Space]),
        (MacOs, &[Meta(Left), Shift(Left), Char('4')]),
        (Windows, &[Ctrl(Left), Char('C')]),
        (Windows, &[Ctrl(Right), Char('V')]),
        (Windows, &[Alt(Left), Key::F(4)]),
        (Windows, &[Alt(Left), Tab]),
        (Windows, &[Meta(Left), Shift(Left), Char('S')]),
        (Linux, &[Ctrl(Left), Char('V')]),
        (Linux, &[Alt(Left), Tab]),
        (Linux, &[Alt(Left), Key::F(4)]),
    ];
    for (platform, keys) in cases {
        assert_eq!(
            check(*platform, Action::PasteLast, keys),
            Err(ShortcutError::Reserved),
            "{platform:?} {keys:?}"
        );
    }
    // Reserved on one OS only.
    assert_eq!(
        check(Windows, Action::PasteLast, &[Meta(Left), Char('C')]),
        Ok(())
    );
    assert_eq!(
        check(MacOs, Action::PasteLast, &[Ctrl(Left), Char('C')]),
        Ok(())
    );
}
