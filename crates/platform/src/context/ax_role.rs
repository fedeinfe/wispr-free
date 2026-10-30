//! Maps a macOS accessibility role to [`Focus`]. Kept OS-free so it is tested everywhere.

use super::Focus;

/// What the AX API says about the focused element.
pub(crate) struct AxElementInfo<'a> {
    pub role: &'a str,
    pub subrole: Option<&'a str>,
    /// `AXValue` is settable and `AXSelectedTextRange` exists: custom editors expose this.
    pub editable_value: bool,
}

const TEXT_ROLES: &[&str] = &["AXTextField", "AXTextArea", "AXComboBox", "AXSearchField"];

/// Native controls that never take typed text. Containers (AXGroup, AXWebArea, AXScrollArea…)
/// are deliberately absent: custom text views hide behind them.
const NON_TEXT_ROLES: &[&str] = &[
    "AXButton",
    "AXCheckBox",
    "AXRadioButton",
    "AXPopUpButton",
    "AXMenuButton",
    "AXDisclosureTriangle",
    "AXSlider",
    "AXIncrementor",
    "AXColorWell",
    "AXList",
    "AXTable",
    "AXOutline",
    "AXBrowser",
    "AXRow",
    "AXCell",
    "AXTabGroup",
    "AXToolbar",
    "AXMenu",
    "AXMenuBar",
    "AXMenuItem",
    "AXLink",
    "AXImage",
];

pub(crate) fn classify(info: &AxElementInfo) -> Focus {
    if info.role == "AXSecureTextField" || info.subrole == Some("AXSecureTextField") {
        Focus::Secure
    } else if TEXT_ROLES.contains(&info.role) || info.editable_value {
        Focus::Editable
    } else if NON_TEXT_ROLES.contains(&info.role) {
        Focus::NotEditable
    } else {
        Focus::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn focus(role: &str, subrole: Option<&str>, editable_value: bool) -> Focus {
        classify(&AxElementInfo {
            role,
            subrole,
            editable_value,
        })
    }

    #[test]
    fn text_roles_and_custom_editors_are_editable() {
        for role in ["AXTextField", "AXTextArea", "AXComboBox", "AXSearchField"] {
            assert_eq!(focus(role, None, false), Focus::Editable, "{role}");
        }
        assert_eq!(focus("AXGroup", None, true), Focus::Editable);
    }

    #[test]
    fn password_fields_are_secure() {
        assert_eq!(
            focus("AXTextField", Some("AXSecureTextField"), true),
            Focus::Secure
        );
    }

    #[test]
    fn plain_controls_are_not_editable() {
        for role in ["AXButton", "AXList", "AXTable", "AXOutline", "AXCheckBox", "AXLink"] {
            assert_eq!(focus(role, None, false), Focus::NotEditable, "{role}");
        }
    }

    #[test]
    fn containers_that_may_host_custom_text_views_stay_unknown() {
        // Electron/Chromium content and terminals often report these; they still get the paste.
        for role in ["AXWebArea", "AXGroup", "AXScrollArea", "AXWindow", "AXUnknown"] {
            assert_eq!(focus(role, None, false), Focus::Unknown, "{role}");
        }
    }
}
