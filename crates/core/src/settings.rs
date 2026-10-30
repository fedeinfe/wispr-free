//! Everything the Settings window can change (spec §5) and the file it lives in. Features read
//! their values from a shared [`SettingsStore`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::shortcut::Bindings;

/// All settings. A file that omits one gets its default, so new settings need no migration.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub general: General,
    pub shortcuts: Bindings,
    pub pipeline: Pipeline,
    pub style: StyleSettings,
    pub meetings: Meetings,
    pub media: Media,
    pub privacy: Privacy,
    pub onboarding: Onboarding,
}

/// Whether the first-run flow was seen through. Delete all resets it with everything else, which
/// is right: what it set up is gone.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Onboarding {
    pub finished: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    /// Input device ids, best first (`wf_audio::CaptureConfig::ranking`). Empty means auto.
    pub microphones: Vec<String>,
    /// ISO 639-1 codes of "My languages". Empty means auto-detect.
    pub languages: Vec<String>,
    /// `"system"` or a UI language code.
    pub app_language: String,
    pub theme: Theme,
    pub launch_at_login: bool,
    pub show_pill: bool,
    /// macOS only.
    pub show_in_dock: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            microphones: Vec::new(),
            languages: Vec::new(),
            app_language: "system".to_owned(),
            theme: Theme::default(),
            launch_at_login: false,
            show_pill: true,
            show_in_dock: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pipeline {
    pub tier: PipelineTier,
    pub denoise: bool,
    pub vad: bool,
    pub llm_cleanup: bool,
    pub cleanup_strength: CleanupStrength,
    pub vad_auto_stop: bool,
    pub vad_auto_stop_seconds: u32,
    /// `None` follows the tier's pick in the model catalog; only Custom names its own models.
    pub asr_model: Option<String>,
    pub vad_model: Option<String>,
    pub llm_model: Option<String>,
    pub keep_models_loaded: KeepModels,
    /// Where models live, perhaps on an external drive. `None` is the app's data folder.
    pub models_folder: Option<PathBuf>,
}

impl Default for Pipeline {
    fn default() -> Self {
        Self {
            tier: PipelineTier::default(),
            denoise: false,
            vad: true,
            llm_cleanup: true,
            cleanup_strength: CleanupStrength::default(),
            vad_auto_stop: false,
            vad_auto_stop_seconds: 3,
            asr_model: None,
            vad_model: None,
            llm_model: None,
            keep_models_loaded: KeepModels::default(),
            models_folder: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineTier {
    Lite,
    #[default]
    Balanced,
    High,
    Custom,
}

/// How far LLM cleanup may depart from the spoken words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupStrength {
    None,
    #[default]
    Light,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeepModels {
    Always,
    #[default]
    FiveMinutes,
    UnloadAfterUse,
}

/// Styles per app category, the dictionary and snippets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StyleSettings {
    pub personal_messages: Style,
    pub work_messages: Style,
    pub email: Style,
    pub other: Style,
    pub dictionary: Vec<String>,
    pub replacements: Vec<Replacement>,
    pub snippets: Vec<Snippet>,
    pub auto_add_corrections: bool,
    /// Saying "press enter" at the end of a dictation presses Enter.
    pub press_enter_command: bool,
}

impl Default for StyleSettings {
    fn default() -> Self {
        Self {
            personal_messages: Style::Casual,
            work_messages: Style::Formal,
            email: Style::Formal,
            other: Style::Formal,
            dictionary: Vec::new(),
            replacements: Vec::new(),
            snippets: Vec::new(),
            auto_add_corrections: true,
            press_enter_command: false,
        }
    }
}

/// Tone and punctuation preset for an app category. LLM cleanup applies a style by rewriting;
/// `wf_pipeline::passes::apply_style` does the mechanical part, so it holds with the LLM off too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    Formal,
    Casual,
    VeryCasual,
    Excited,
}

/// A dictionary replacement: `wrong` is always written as `right`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replacement {
    pub wrong: String,
    pub right: String,
}

/// A user snippet: saying `trigger` writes `text`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    pub trigger: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Meetings {
    /// Suggest taking notes when a meeting app starts using the mic. Never records by itself.
    pub auto_detect: bool,
    pub live_transcript: bool,
    pub refine_pass: bool,
    pub diarization: bool,
    pub keep_audio: bool,
    pub max_length_minutes: u32,
    /// Extra instructions for the summary. Empty means the standard sections.
    pub summary_template: String,
}

impl Default for Meetings {
    fn default() -> Self {
        Self {
            auto_detect: false,
            live_transcript: true,
            refine_pass: true,
            diarization: true,
            keep_audio: true,
            max_length_minutes: 240,
            summary_template: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Media {
    pub sounds: bool,
    pub while_dictating: MediaWhileDictating,
}

impl Default for Media {
    fn default() -> Self {
        Self {
            sounds: true,
            while_dictating: MediaWhileDictating::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaWhileDictating {
    #[default]
    Nothing,
    Mute,
    Pause,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Privacy {
    pub context_awareness: bool,
    pub history_retention: HistoryRetention,
}

impl Default for Privacy {
    fn default() -> Self {
        Self {
            context_awareness: true,
            history_retention: HistoryRetention::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryRetention {
    #[default]
    Keep,
    /// Entries are deleted once they are 24 hours old.
    Delete24h,
    /// Nothing reaches the disk.
    Never,
}

/// The settings of this run, backed by a TOML file. Every update is written through at once, so
/// a crash loses nothing.
#[derive(Debug)]
pub struct SettingsStore {
    path: PathBuf,
    current: Mutex<Settings>,
    set_aside: Option<PathBuf>,
}

impl SettingsStore {
    /// Where the app keeps its settings.
    pub fn default_path() -> Option<PathBuf> {
        Some(project_dirs()?.config_dir().join("settings.toml"))
    }

    /// Reads `path`. A missing file means defaults. A damaged one is moved next to it so that the
    /// next save can't destroy it, and defaults are used; see [`Self::set_aside`].
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let (current, set_aside) = match fs::read(&path) {
            Ok(bytes) => match parse(&bytes) {
                Some(settings) => (settings, None),
                None => (Settings::default(), move_aside(&path)),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => (Settings::default(), None),
            Err(_) => (Settings::default(), move_aside(&path)),
        };
        Self {
            path,
            current: Mutex::new(current),
            set_aside,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Where the damaged file went, if this run started from one.
    pub fn set_aside(&self) -> Option<&Path> {
        self.set_aside.as_deref()
    }

    pub fn get(&self) -> Settings {
        self.lock().clone()
    }

    /// Changes the settings and saves them. If saving fails the change still holds for this run.
    pub fn update(&self, change: impl FnOnce(&mut Settings)) -> io::Result<()> {
        let mut current = self.lock();
        change(&mut current);
        save(&self.path, &current)
    }

    fn lock(&self) -> MutexGuard<'_, Settings> {
        self.current.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The folder for models, history and meetings. Settings may live elsewhere; see
/// [`SettingsStore::default_path`].
pub fn data_dir() -> Option<PathBuf> {
    Some(project_dirs()?.data_dir().to_path_buf())
}

fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", "wispr-free")
}

fn parse(bytes: &[u8]) -> Option<Settings> {
    toml::from_str(std::str::from_utf8(bytes).ok()?).ok()
}

fn move_aside(path: &Path) -> Option<PathBuf> {
    let mut target = path.as_os_str().to_owned();
    target.push(".bad");
    let target = PathBuf::from(target);
    fs::rename(path, &target).ok().map(|()| target)
}

/// Writes through a temporary file, so a crash never leaves half a settings file.
fn save(path: &Path, settings: &Settings) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(settings).map_err(io::Error::other)?;
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    fs::write(&temporary, text)?;
    fs::rename(&temporary, path)
}
