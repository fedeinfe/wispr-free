//! Wayland has no XTest, so the paste chord goes through wtype or ydotool. Argument building
//! is kept OS-free so it is tested everywhere.

use super::PasteChord;

pub(crate) fn wtype_args(chord: PasteChord) -> Vec<&'static str> {
    match chord {
        PasteChord::ShiftInsert => vec!["-M", "shift", "-k", "Insert", "-m", "shift"],
        PasteChord::CtrlV | PasteChord::CmdV => vec!["-M", "ctrl", "-k", "v", "-m", "ctrl"],
    }
}

/// Linux input key codes: KEY_LEFTSHIFT 42, KEY_INSERT 110, KEY_LEFTCTRL 29, KEY_V 47.
pub(crate) fn ydotool_args(chord: PasteChord) -> Vec<&'static str> {
    match chord {
        PasteChord::ShiftInsert => vec!["key", "42:1", "110:1", "110:0", "42:0"],
        PasteChord::CtrlV | PasteChord::CmdV => vec!["key", "29:1", "47:1", "47:0", "29:0"],
    }
}

pub(crate) fn wtype_enter_args() -> Vec<&'static str> {
    vec!["-k", "Return"]
}

/// KEY_ENTER 28.
pub(crate) fn ydotool_enter_args() -> Vec<&'static str> {
    vec!["key", "28:1", "28:0"]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wtype_holds_the_modifier_around_the_key() {
        assert_eq!(
            wtype_args(PasteChord::CtrlV),
            ["-M", "ctrl", "-k", "v", "-m", "ctrl"]
        );
        assert_eq!(
            wtype_args(PasteChord::ShiftInsert),
            ["-M", "shift", "-k", "Insert", "-m", "shift"]
        );
    }

    #[test]
    fn enter_is_a_plain_key_press_on_both_tools() {
        assert_eq!(wtype_enter_args(), ["-k", "Return"]);
        assert_eq!(ydotool_enter_args(), ["key", "28:1", "28:0"]);
    }

    #[test]
    fn ydotool_sends_linux_input_key_codes() {
        // KEY_LEFTCTRL 29, KEY_V 47, KEY_LEFTSHIFT 42, KEY_INSERT 110; :1 press, :0 release.
        assert_eq!(
            ydotool_args(PasteChord::CtrlV),
            ["key", "29:1", "47:1", "47:0", "29:0"]
        );
        assert_eq!(
            ydotool_args(PasteChord::ShiftInsert),
            ["key", "42:1", "110:1", "110:0", "42:0"]
        );
    }
}
