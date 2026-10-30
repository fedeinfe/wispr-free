//! How shortcuts look in the settings file and in the Settings window.

use wf_core::shortcut::Key::{self, Alt, Char, Ctrl, F, Meta, Mouse, Shift, Space};
use wf_core::shortcut::Platform::{Linux, MacOs, Windows};
use wf_core::shortcut::Side::{Either, Left, Right};
use wf_core::shortcut::{Action, Bindings, Combo, MouseButton};

#[test]
fn every_kind_of_key_has_a_stable_name_in_the_settings_file() {
    let cases: &[(Key, &str)] = &[
        (Key::Fn, "Fn"),
        (Ctrl(Either), "Ctrl"),
        (Ctrl(Left), "LeftCtrl"),
        (Shift(Right), "RightShift"),
        (Alt(Either), "Alt"),
        (Meta(Right), "RightMeta"),
        (Key::Esc, "Esc"),
        (Space, "Space"),
        (Key::ArrowUp, "ArrowUp"),
        (Key::PageDown, "PageDown"),
        (Key::PrintScreen, "PrintScreen"),
        (Char('V'), "V"),
        (Char('`'), "`"),
        (F(13), "F13"),
        (Mouse(MouseButton::Middle), "MouseMiddle"),
        (Mouse(MouseButton::Back), "MouseBack"),
        (Mouse(MouseButton::Forward), "MouseForward"),
        (Mouse(MouseButton::Other(7)), "Mouse7"),
        (Key::Other(200), "Code200"),
    ];
    for &(key, name) in cases {
        assert_eq!(key.to_string(), name);
        assert_eq!(name.parse::<Key>(), Ok(key), "{name}");
    }
}

#[test]
fn a_name_that_is_not_a_key_is_refused() {
    for name in [
        "", "Hyper", "F0", "F25", "AB", "a", "Mouse", "Code", "LeftFn",
    ] {
        assert!(name.parse::<Key>().is_err(), "{name:?}");
    }
}

#[test]
fn the_defaults_read_like_the_table_in_the_spec() {
    let cases = [
        (
            MacOs,
            ["Fn", "Fn+Space", "Esc", "Ctrl+Cmd+V", "Ctrl+Cmd+C", "Opt+M"],
        ),
        (
            Windows,
            [
                "Ctrl+Win",
                "Ctrl+Win+Space",
                "Esc",
                "Shift+Alt+Z",
                "Shift+Alt+X",
                "Win+Alt+M",
            ],
        ),
        (
            Linux,
            [
                "Ctrl+Super",
                "Ctrl+Super+Space",
                "Esc",
                "Shift+Alt+Z",
                "Shift+Alt+X",
                "Super+Alt+M",
            ],
        ),
    ];
    for (platform, labels) in cases {
        let bindings = Bindings::default_for(platform);
        for (action, label) in Action::ALL.into_iter().zip(labels) {
            assert_eq!(bindings.get(action).label(platform), label, "{action:?}");
        }
    }
}

#[test]
fn a_sided_modifier_says_which_side() {
    assert_eq!(Combo::new([Meta(Right)]).label(MacOs), "Right Cmd");
    assert_eq!(Combo::new([Alt(Left), F(5)]).label(Windows), "Left Alt+F5");
    assert_eq!(
        Combo::new([Mouse(MouseButton::Back)]).label(Linux),
        "Mouse Back"
    );
    assert_eq!(Combo::new([]).label(MacOs), "");
}
