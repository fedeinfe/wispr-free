//! The pill's menu and the status menu, as data. A front end draws them and hands back the action
//! of the item picked, so every title and every rule about what's there stays here.

use wf_audio::InputDevice;
use wf_core::pill::{PillState, Sound};
use wf_core::settings::PipelineTier;

use crate::settings::rows::LANGUAGES;
use crate::settings::values::TIERS;

const MENU_LABEL_CHARS: usize = 48;
/// The names of [`TIERS`], in its order.
const PIPELINE_QUALITIES: [&str; 4] = ["Lite", "Balanced", "High", "Custom"];

/// What an item does. Anything that needs a window comes back as a [`crate::bridge::ShellEvent`].
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum MenuAction {
    PasteLast,
    PasteRecent {
        index: u32,
    },
    Microphone {
        id: String,
    },
    /// `None` is Auto-detect. It replaces My languages, as it always has.
    Language {
        code: Option<String>,
    },
    PipelineQuality {
        tier: PipelineTier,
    },
    ToggleMeeting,
    OpenMeetings,
    OpenOnboarding,
    OpenSettings,
    HideForAnHour,
    Quit,
    DebugState {
        state: PillState,
    },
    DebugSound {
        sound: Sound,
    },
    DebugMicLevel,
}

/// One row of a menu. A submenu holds rows of the same kind.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum MenuEntry {
    Item {
        title: String,
        action: MenuAction,
    },
    Submenu {
        title: String,
        entries: Vec<MenuEntry>,
    },
    Separator,
}

fn item(title: impl Into<String>, action: MenuAction) -> MenuEntry {
    MenuEntry::Item {
        title: title.into(),
        action,
    }
}

fn submenu(title: &str, entries: Vec<MenuEntry>) -> MenuEntry {
    MenuEntry::Submenu {
        title: title.to_owned(),
        entries,
    }
}

/// The menu of the pill.
pub fn pill_menu(recent: &[String], microphones: &[InputDevice]) -> Vec<MenuEntry> {
    let recent = recent
        .iter()
        .zip(0..)
        .map(|(text, index)| item(menu_label(text), MenuAction::PasteRecent { index }));
    let microphones = microphones.iter().map(|device| {
        item(
            device.name.as_str(),
            MenuAction::Microphone {
                id: device.id.clone(),
            },
        )
    });
    let auto_detect = item("Auto-detect", MenuAction::Language { code: None });
    let languages = LANGUAGES.iter().map(|&(code, name)| {
        item(
            name,
            MenuAction::Language {
                code: Some(code.to_owned()),
            },
        )
    });
    let qualities = TIERS
        .into_iter()
        .zip(PIPELINE_QUALITIES)
        .map(|(tier, name)| item(name, MenuAction::PipelineQuality { tier }));
    vec![
        item("Paste last transcript", MenuAction::PasteLast),
        submenu("Recent transcripts", recent.collect()),
        submenu("Microphone", microphones.collect()),
        submenu(
            "Language",
            std::iter::once(auto_detect).chain(languages).collect(),
        ),
        submenu("Pipeline quality", qualities.collect()),
        MenuEntry::Separator,
        item("Meetings", MenuAction::OpenMeetings),
        item("Settings", MenuAction::OpenSettings),
        item("Hide for 1 hour", MenuAction::HideForAnHour),
        item("Quit", MenuAction::Quit),
    ]
}

/// The menu of the status item. `setup_needed` keeps a way back to onboarding, and `debug` adds what
/// reaches every pill state by hand.
pub fn status_menu(in_meeting: bool, setup_needed: bool, debug: bool) -> Vec<MenuEntry> {
    let meeting = if in_meeting {
        "Stop meeting notes"
    } else {
        "Start meeting notes"
    };
    let mut entries = Vec::new();
    if setup_needed {
        entries.push(item("Finish setup…", MenuAction::OpenOnboarding));
    }
    entries.push(item(meeting, MenuAction::ToggleMeeting));
    if debug {
        entries.push(submenu("Debug pill", debug_entries()));
    }
    entries.extend([
        item("Meetings…", MenuAction::OpenMeetings),
        item("Settings…", MenuAction::OpenSettings),
        item("Quit wispr-free", MenuAction::Quit),
    ]);
    entries
}

fn debug_entries() -> Vec<MenuEntry> {
    let state = |title, state| item(title, MenuAction::DebugState { state });
    let sound = |title, sound| item(title, MenuAction::DebugSound { sound });
    vec![
        state("Hidden", PillState::Hidden),
        state("Resting", PillState::Resting),
        state("Ready", PillState::Ready),
        state("Listening (push-to-talk)", PillState::ListeningPtt),
        state("Listening (hands-free)", PillState::ListeningHandsFree),
        state("Processing", PillState::Processing),
        state(
            "Error",
            PillState::Error {
                reason: "No microphone found".into(),
            },
        ),
        state("Meeting", PillState::Meeting),
        MenuEntry::Separator,
        sound("Play start sound", Sound::Start),
        sound("Play stop sound", Sound::Stop),
        sound("Play lock sound", Sound::Lock),
        sound("Play error sound", Sound::Error),
        MenuEntry::Separator,
        item("Use real mic level", MenuAction::DebugMicLevel),
    ]
}

/// One line of a transcript, short enough for a menu.
pub fn menu_label(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default();
    match line.char_indices().nth(MENU_LABEL_CHARS) {
        Some((end, _)) => format!("{}…", &line[..end]),
        None => line.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(entries: &[MenuEntry]) -> Vec<&str> {
        entries
            .iter()
            .map(|entry| match entry {
                MenuEntry::Item { title, .. } | MenuEntry::Submenu { title, .. } => title.as_str(),
                MenuEntry::Separator => "-",
            })
            .collect()
    }

    fn submenu_of<'a>(entries: &'a [MenuEntry], name: &str) -> &'a [MenuEntry] {
        entries
            .iter()
            .find_map(|entry| match entry {
                MenuEntry::Submenu { title, entries } if title == name => Some(entries.as_slice()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no {name}"))
    }

    #[test]
    fn the_pill_menu_keeps_its_items_and_order() {
        let menu = pill_menu(&[], &[]);

        assert_eq!(
            titles(&menu),
            [
                "Paste last transcript",
                "Recent transcripts",
                "Microphone",
                "Language",
                "Pipeline quality",
                "-",
                "Meetings",
                "Settings",
                "Hide for 1 hour",
                "Quit",
            ]
        );
    }

    #[test]
    fn the_language_menu_is_auto_detect_and_the_one_list_of_fifteen() {
        let menu = pill_menu(&[], &[]);
        let languages = submenu_of(&menu, "Language");

        assert_eq!(languages.len(), 1 + LANGUAGES.len());
        assert_eq!(
            languages[0],
            item("Auto-detect", MenuAction::Language { code: None })
        );
        assert_eq!(
            languages[2],
            item(
                "Italian",
                MenuAction::Language {
                    code: Some("it".to_owned())
                }
            )
        );
    }

    #[test]
    fn pipeline_quality_names_each_tier_it_sets() {
        let menu = pill_menu(&[], &[]);

        assert_eq!(
            submenu_of(&menu, "Pipeline quality"),
            [
                item(
                    "Lite",
                    MenuAction::PipelineQuality {
                        tier: PipelineTier::Lite
                    }
                ),
                item(
                    "Balanced",
                    MenuAction::PipelineQuality {
                        tier: PipelineTier::Balanced
                    }
                ),
                item(
                    "High",
                    MenuAction::PipelineQuality {
                        tier: PipelineTier::High
                    }
                ),
                item(
                    "Custom",
                    MenuAction::PipelineQuality {
                        tier: PipelineTier::Custom
                    }
                ),
            ]
        );
    }

    #[test]
    fn recent_transcripts_show_their_first_line_short_and_keep_their_place() {
        let long = "x".repeat(60);
        let recent = ["first line\nsecond".to_owned(), long];
        let menu = pill_menu(&recent, &[]);

        assert_eq!(
            submenu_of(&menu, "Recent transcripts"),
            [
                item("first line", MenuAction::PasteRecent { index: 0 }),
                item(
                    format!("{}…", "x".repeat(48)),
                    MenuAction::PasteRecent { index: 1 }
                ),
            ]
        );
    }

    #[test]
    fn a_microphone_is_shown_by_name_and_chosen_by_id() {
        let device = InputDevice {
            id: "usb-1".to_owned(),
            name: "Desk mic".to_owned(),
        };
        let menu = pill_menu(&[], &[device]);

        assert_eq!(
            submenu_of(&menu, "Microphone"),
            [item(
                "Desk mic",
                MenuAction::Microphone {
                    id: "usb-1".to_owned()
                }
            )]
        );
    }

    #[test]
    fn the_status_menu_says_what_the_meeting_button_will_do() {
        assert_eq!(
            titles(&status_menu(false, false, false)),
            [
                "Start meeting notes",
                "Meetings…",
                "Settings…",
                "Quit wispr-free"
            ]
        );
        assert_eq!(
            titles(&status_menu(true, false, false))[0],
            "Stop meeting notes"
        );
    }

    #[test]
    fn finish_setup_comes_first_and_only_when_setup_is_needed() {
        assert_eq!(
            status_menu(false, true, false)[0],
            item("Finish setup…", MenuAction::OpenOnboarding)
        );
    }

    #[test]
    fn the_debug_menu_reaches_every_state_and_sound() {
        let menu = status_menu(false, false, true);

        assert_eq!(titles(&menu)[1], "Debug pill");
        assert_eq!(
            titles(submenu_of(&menu, "Debug pill")),
            [
                "Hidden",
                "Resting",
                "Ready",
                "Listening (push-to-talk)",
                "Listening (hands-free)",
                "Processing",
                "Error",
                "Meeting",
                "-",
                "Play start sound",
                "Play stop sound",
                "Play lock sound",
                "Play error sound",
                "-",
                "Use real mic level",
            ]
        );
    }
}
