//! Where the pill was left and which page Settings was on. Its `sounds` follows the Sounds setting
//! once the app has started.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use wf_core::pill::dock::Edge;

use crate::settings::page::SettingsPage;

const FILE_NAME: &str = "pill.toml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefs {
    pub edge: Edge,
    pub sounds: bool,
    pub settings_page: SettingsPage,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            edge: Edge::default(),
            sounds: true,
            settings_page: SettingsPage::default(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Stored {
    edge: String,
    sounds: bool,
    #[serde(default)]
    settings_page: String,
}

impl Prefs {
    /// Reads `path`, falling back to the defaults for a missing or unreadable file.
    pub fn load(path: &Path) -> Self {
        let stored = fs::read_to_string(path)
            .ok()
            .and_then(|text| toml::from_str::<Stored>(&text).ok());
        stored.map_or_else(Self::default, |stored| Self {
            edge: stored.edge.parse().unwrap_or_default(),
            sounds: stored.sounds,
            settings_page: stored.settings_page.parse().unwrap_or_default(),
        })
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let stored = Stored {
            edge: self.edge.as_str().to_owned(),
            sounds: self.sounds,
            settings_page: self.settings_page.as_str().to_owned(),
        };
        let text = toml::to_string(&stored).map_err(io::Error::other)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, text)
    }
}

pub fn path() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "wispr-free")?;
    Some(dirs.config_dir().join(FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wf-prefs-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("nested").join(FILE_NAME)
    }

    #[test]
    fn everything_saved_comes_back() {
        let path = temp_file("round-trip");
        let prefs = Prefs {
            edge: Edge::Right,
            sounds: false,
            settings_page: SettingsPage::Privacy,
        };
        prefs.save(&path).unwrap();
        assert_eq!(Prefs::load(&path), prefs);
    }

    #[test]
    fn a_missing_file_gives_the_bottom_edge_with_sounds_on() {
        let prefs = Prefs::load(&temp_file("missing"));
        assert_eq!(prefs.edge, Edge::Bottom);
        assert!(prefs.sounds);
    }

    #[test]
    fn a_corrupt_file_gives_the_defaults() {
        let path = temp_file("corrupt");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "edge = = nonsense").unwrap();
        assert_eq!(Prefs::load(&path), Prefs::default());
    }

    #[test]
    fn a_file_from_before_the_page_was_kept_opens_on_the_first_page() {
        let path = temp_file("no-page");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "edge = \"left\"\nsounds = false\n").unwrap();
        let prefs = Prefs::load(&path);
        assert_eq!(prefs.edge, Edge::Left);
        assert_eq!(prefs.settings_page, SettingsPage::General);
    }

    #[test]
    fn an_unknown_page_falls_back_to_the_first_and_keeps_the_rest() {
        let path = temp_file("unknown-page");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "edge = \"right\"\nsounds = false\nsettings_page = \"gone\"\n",
        )
        .unwrap();
        let prefs = Prefs::load(&path);
        assert_eq!(prefs.edge, Edge::Right);
        assert_eq!(prefs.settings_page, SettingsPage::General);
    }

    #[test]
    fn an_unknown_edge_falls_back_to_the_bottom_and_keeps_the_rest() {
        let path = temp_file("unknown-edge");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "edge = \"top\"\nsounds = false\n").unwrap();
        let prefs = Prefs::load(&path);
        assert_eq!(prefs.edge, Edge::Bottom);
        assert!(!prefs.sounds);
    }
}
