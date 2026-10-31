//! Where a run keeps its files. `Shell` gets every one of them from its caller, or from
//! [`real_paths`] once the run is proven to be the real app: `ProjectDirs` names the user's real
//! folder by itself, so a process with its own bundle id would still land in it.

use std::path::{Component, Path, PathBuf};

use wf_core::settings::{SettingsStore, data_dir};
use wf_dictation::remote::engine_host_beside;
use wf_history::History;
use wf_models::{ModelStore, resolved};

use crate::bridge::ShellError;
use crate::launch::Run;
use crate::prefs;
use crate::settings::system::export_folder;

/// The dev app's folder, beside the real one.
const DEV_ROOT: &str = "wispr-free-dev";

uniffi::use_remote_type!(wf_core::PathBuf);

/// Every path a run touches.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Paths {
    /// The run's own folder. Delete all removes it, and on a dev run no model changes outside it
    /// (see [`Paths::model_fence`]).
    pub data: PathBuf,
    pub settings: PathBuf,
    /// Where the pill was left.
    pub pill: PathBuf,
    /// Where models live unless the settings name a folder. One the settings name may be anywhere,
    /// which is how the real app reaches an external drive and a dev run reads what's on disk.
    pub models: PathBuf,
    /// The media note goes beside it.
    pub history: PathBuf,
    pub meetings: PathBuf,
    /// Where Export all puts its folder.
    pub exports: PathBuf,
    pub engine_host: PathBuf,
}

impl Paths {
    pub fn under(root: &Path, engine_host: PathBuf) -> Self {
        Self {
            data: root.to_owned(),
            settings: root.join("settings.toml"),
            pill: root.join("pill.toml"),
            models: root.join("models"),
            history: root.join("history.sqlite3"),
            meetings: root.join("meetings"),
            exports: root.join("exports"),
            engine_host,
        }
    }

    /// Refuses a path that isn't plain, and one inside any of `real_folders`, also by way of a link.
    pub fn check(&self, real_folders: &[PathBuf]) -> Result<(), ShellError> {
        let real_folders: Vec<_> = real_folders.iter().map(|real| resolved(real)).collect();
        for path in self.all() {
            if !is_plain(path) {
                return Err(ShellError::UnclearPath {
                    path: path.display().to_string(),
                });
            }
            let real = resolved(path);
            if real_folders.iter().any(|folder| real.starts_with(folder)) {
                return Err(ShellError::RealData {
                    path: path.display().to_string(),
                });
            }
        }
        Ok(())
    }

    /// The one folder a run may change models in, if it's fenced. A dev run may read models
    /// anywhere its settings say, but the user's real models folder is named only in their real
    /// settings, which a dev run can't read. So this says what's allowed rather than what isn't.
    /// The real app owns its models wherever they are.
    pub fn model_fence(&self, run: Run) -> Option<PathBuf> {
        match run {
            Run::Real => None,
            Run::Dev => Some(self.data.clone()),
        }
    }

    fn all(&self) -> [&PathBuf; 8] {
        [
            &self.data,
            &self.settings,
            &self.pill,
            &self.models,
            &self.history,
            &self.meetings,
            &self.exports,
            &self.engine_host,
        ]
    }
}

/// Absolute with nothing to resolve, so comparing it says where it really points.
fn is_plain(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|part| !matches!(part, Component::ParentDir | Component::CurDir))
}

/// The folders that hold the user's own data, by the names the other crates hard-code. A dev run
/// may start on none of them.
pub(crate) fn real_folders() -> Vec<PathBuf> {
    let settings = SettingsStore::default_path().and_then(|path| Some(path.parent()?.to_owned()));
    data_dir().into_iter().chain(settings).collect()
}

/// The real app's paths: where the app has always kept the user's data, by those same names.
pub fn real_paths() -> Result<Paths, ShellError> {
    let data = data_dir().ok_or(ShellError::NoHome)?;
    let executable = std::env::current_exe().map_err(|_| ShellError::NoHome)?;
    Ok(Paths {
        settings: SettingsStore::default_path().ok_or(ShellError::NoHome)?,
        pill: prefs::path().ok_or(ShellError::NoHome)?,
        models: ModelStore::default_root().ok_or(ShellError::NoHome)?,
        history: History::default_path().ok_or(ShellError::NoHome)?,
        meetings: data.join("meetings"),
        exports: export_folder(&data),
        engine_host: engine_host_beside(&executable),
        data,
    })
}

/// The dev app's paths. Fails rather than guess when there's no home folder to put them in.
#[uniffi::export]
pub fn dev_paths() -> Result<Paths, ShellError> {
    let dirs = directories::ProjectDirs::from("", "", DEV_ROOT).ok_or(ShellError::NoHome)?;
    let executable = std::env::current_exe().map_err(|_| ShellError::NoHome)?;
    Ok(Paths::under(
        dirs.data_dir(),
        engine_host_beside(&executable),
    ))
}
