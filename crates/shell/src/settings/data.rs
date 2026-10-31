//! Export all and delete all (Privacy & data).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Folder under the data folder that is not exported: models can be downloaded again.
const MODELS: &str = "models";

/// Copies the settings and everything in `data_dir` except models into a new folder in
/// `export_dir`, and returns that folder.
pub fn export_all(data_dir: &Path, settings: &Path, export_dir: &Path) -> io::Result<PathBuf> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let target = export_dir.join(format!("wispr-free-export-{stamp}"));
    fs::create_dir_all(&target)?;
    if data_dir.exists() {
        for entry in fs::read_dir(data_dir)? {
            let entry = entry?;
            if entry.file_name() != MODELS {
                copy_tree(&entry.path(), &target.join(entry.file_name()))?;
            }
        }
    }
    if settings.exists() {
        fs::copy(settings, target.join("settings.toml"))?;
    }
    Ok(target)
}

/// Symlinks are skipped: nothing the app writes is one, and following them could leave the folder.
fn copy_tree(source: &Path, target: &Path) -> io::Result<()> {
    let kind = fs::symlink_metadata(source)?.file_type();
    if kind.is_dir() {
        fs::create_dir_all(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(&entry.path(), &target.join(entry.file_name()))?;
        }
    } else if kind.is_file() {
        fs::copy(source, target)?;
    }
    Ok(())
}

/// Removes the data folder and everything in it. Fine if it never existed.
pub fn delete_all(data_dir: &Path) -> io::Result<()> {
    match fs::remove_dir_all(data_dir) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}
