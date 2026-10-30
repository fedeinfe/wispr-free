//! Maps a UI Automation control type to [`Focus`]. Kept OS-free so it is tested everywhere.

use super::Focus;

/// UIA_ControlTypeIds: ComboBox, Edit, Document.
const TEXT_CONTROLS: &[i32] = &[50003, 50004, 50030];

/// UIA_ControlTypeIds of controls that never take typed text: Button, Calendar, CheckBox,
/// Hyperlink, Image, ListItem, List, Menu, MenuBar, MenuItem, ProgressBar, RadioButton,
/// ScrollBar, Slider, Spinner, Tab, TabItem, ToolBar, Tree, TreeItem, DataGrid, DataItem,
/// SplitButton, Table. Containers (Pane, Group, Custom, Window) are deliberately absent:
/// custom text views hide behind them.
const NON_TEXT_CONTROLS: &[i32] = &[
    50000, 50001, 50002, 50005, 50006, 50007, 50008, 50009, 50010, 50011, 50012, 50013, 50014,
    50015, 50016, 50018, 50019, 50021, 50023, 50024, 50028, 50029, 50031, 50036,
];

pub(crate) fn classify(control_type: i32, is_password: bool) -> Focus {
    if is_password {
        Focus::Secure
    } else if TEXT_CONTROLS.contains(&control_type) {
        Focus::Editable
    } else if NON_TEXT_CONTROLS.contains(&control_type) {
        Focus::NotEditable
    } else {
        Focus::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_and_documents_are_editable_and_passwords_secure() {
        assert_eq!(classify(50004, false), Focus::Editable); // Edit
        assert_eq!(classify(50030, false), Focus::Editable); // Document
        assert_eq!(classify(50003, false), Focus::Editable); // ComboBox
        assert_eq!(classify(50004, true), Focus::Secure);
    }

    #[test]
    fn plain_controls_are_not_editable() {
        for control_type in [50000, 50002, 50008, 50007, 50023, 50011, 50005] {
            assert_eq!(classify(control_type, false), Focus::NotEditable);
        }
    }

    #[test]
    fn containers_that_may_host_custom_text_views_stay_unknown() {
        // Pane, Custom, Group, Window: terminals and custom editors report these.
        for control_type in [50033, 50025, 50026, 50032] {
            assert_eq!(classify(control_type, false), Focus::Unknown);
        }
    }
}
