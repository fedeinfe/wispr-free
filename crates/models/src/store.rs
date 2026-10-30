//! Installed models on disk: `<root>/<model id>/<file name>`. Downloads stream into
//! `<file name>.part`, are checked against the manifest's size and SHA-256, then renamed into
//! place, so a file under its final name is always complete.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::{Catalog, ModelEntry, ModelFile, Tier};

const PART_SUFFIX: &str = ".part";
const CHUNK: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelStatus {
    NotInstalled,
    /// Some bytes are on disk (finished files or `.part` leftovers); download resumes from here.
    Partial {
        downloaded: u64,
    },
    /// Every file is present with its manifest size. [`ModelStore::verify`] checks the hashes.
    Installed,
}

/// Bytes on disk for the whole model, across its files.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Progress {
    pub downloaded: u64,
    pub total: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("download cancelled")]
    Cancelled,
    #[error("{file}: checksum mismatch (expected {expected}, got {actual})")]
    ChecksumMismatch {
        file: String,
        expected: String,
        actual: String,
    },
    #[error("{file}: server sent {actual} bytes, manifest says {expected}")]
    SizeMismatch {
        file: String,
        expected: u64,
        actual: u64,
    },
    #[error("{file}: {source}")]
    Http {
        file: String,
        source: Box<ureq::Error>,
    },
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub struct ModelStore {
    root: PathBuf,
    agent: ureq::Agent,
}

impl ModelStore {
    pub fn new(root: impl Into<PathBuf>) -> ModelStore {
        let agent = ureq::Agent::config_builder()
            .user_agent(concat!("wispr-free/", env!("CARGO_PKG_VERSION")))
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            // A stalled transfer fails instead of hanging; the next attempt resumes.
            .timeout_per_read(Some(Duration::from_secs(60)))
            .build()
            .new_agent();
        ModelStore {
            root: root.into(),
            agent,
        }
    }

    /// `models/` in the per-user app data folder.
    pub fn default_root() -> Option<PathBuf> {
        let dirs = directories::ProjectDirs::from("", "", "wispr-free")?;
        Some(dirs.data_dir().join("models"))
    }

    pub fn model_dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    pub fn status(&self, model: &ModelEntry) -> ModelStatus {
        let dir = self.model_dir(&model.id);
        let mut complete = 0;
        let mut downloaded = 0;
        for file in &model.files {
            if file_len(&dir.join(&file.name)) == Some(file.size) {
                complete += 1;
                downloaded += file.size;
            } else {
                downloaded += file_len(&part_path(&dir, file)).unwrap_or(0).min(file.size);
            }
        }
        if complete == model.files.len() {
            ModelStatus::Installed
        } else if downloaded > 0 {
            ModelStatus::Partial { downloaded }
        } else {
            ModelStatus::NotInstalled
        }
    }

    /// Re-hashes every installed file against the manifest. `false` when a file is missing or
    /// corrupt; downloading again replaces only what's missing, so delete a corrupt model first.
    pub fn verify(&self, model: &ModelEntry) -> io::Result<bool> {
        let dir = self.model_dir(&model.id);
        for file in &model.files {
            let path = dir.join(&file.name);
            if file_len(&path) != Some(file.size) {
                return Ok(false);
            }
            let mut hasher = Sha256::new();
            hash_into(&path, &mut hasher)?;
            if hex(&hasher.finalize()) != file.sha256 {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Removes the model's folder, including partial downloads.
    pub fn delete(&self, model: &ModelEntry) -> io::Result<()> {
        match fs::remove_dir_all(self.model_dir(&model.id)) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }

    /// Downloads whatever is missing. Blocking: run it on a worker thread. Setting `cancel`
    /// stops between chunks and keeps the partial file for a later resume.
    pub fn download(
        &self,
        model: &ModelEntry,
        cancel: &AtomicBool,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<(), DownloadError> {
        let dir = self.model_dir(&model.id);
        fs::create_dir_all(&dir)?;
        let total = model.download_size();
        let mut done = 0;
        for file in &model.files {
            let target = dir.join(&file.name);
            if file_len(&target) != Some(file.size) {
                self.fetch(file, &dir, cancel, &mut |n| {
                    on_progress(Progress {
                        downloaded: done + n,
                        total,
                    })
                })?;
            }
            done += file.size;
            on_progress(Progress {
                downloaded: done,
                total,
            });
        }
        Ok(())
    }

    /// Downloads every model the tier names that isn't installed yet. Progress covers the tier's
    /// whole download size, so a resumed or upgraded install doesn't start from zero.
    pub fn install_tier(
        &self,
        catalog: &Catalog,
        tier: Tier,
        cancel: &AtomicBool,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<(), DownloadError> {
        let models = catalog.models_for_tier(tier);
        let total = models.iter().map(|m| m.download_size()).sum();
        let mut done = 0;
        for model in models {
            self.download(model, cancel, &mut |p| {
                on_progress(Progress {
                    downloaded: done + p.downloaded,
                    total,
                })
            })?;
            done += model.download_size();
        }
        Ok(())
    }

    /// Downloads one file to `<name>.part`, verifies it and renames it into place.
    /// `on_bytes` gets the file's bytes on disk so far.
    fn fetch(
        &self,
        file: &ModelFile,
        dir: &Path,
        cancel: &AtomicBool,
        on_bytes: &mut dyn FnMut(u64),
    ) -> Result<(), DownloadError> {
        let part = part_path(dir, file);
        let http = |source| DownloadError::Http {
            file: file.name.clone(),
            source: Box::new(source),
        };
        let mut hasher = Sha256::new();
        let mut written = match file_len(&part) {
            Some(len) if len <= file.size => hash_into(&part, &mut hasher)?,
            _ => 0,
        };
        if written == file.size {
            return finish(file, &part, dir, written, hasher);
        }

        // ADR-0004: a redirect can't downgrade an https download to plain http.
        let https_only = file.url.starts_with("https://");
        let mut request = self
            .agent
            .get(&file.url)
            .config()
            .https_only(https_only)
            .build();
        if written > 0 {
            request = request.header("Range", format!("bytes={written}-"));
        }
        let mut response = request.call().map_err(http)?;
        let resumed = written > 0
            && response.status() == 206
            && content_range_start(response.headers()) == Some(written);
        let mut out = if resumed {
            OpenOptions::new().append(true).open(&part)?
        } else {
            // The server sent the whole file (no range support): start over.
            hasher = Sha256::new();
            written = 0;
            File::create(&part)?
        };
        let mut body = response.body_mut().as_reader();
        let mut buf = vec![0; CHUNK];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(DownloadError::Cancelled);
            }
            let n = body.read(&mut buf)?;
            if n == 0 {
                break;
            }
            if written + n as u64 > file.size {
                drop(out);
                fs::remove_file(&part)?;
                return Err(DownloadError::SizeMismatch {
                    file: file.name.clone(),
                    expected: file.size,
                    actual: written + n as u64,
                });
            }
            out.write_all(&buf[..n])?;
            hasher.update(&buf[..n]);
            written += n as u64;
            on_bytes(written);
        }
        out.sync_all()?;
        drop(out);
        finish(file, &part, dir, written, hasher)
    }
}

fn finish(
    file: &ModelFile,
    part: &Path,
    dir: &Path,
    len: u64,
    hasher: Sha256,
) -> Result<(), DownloadError> {
    if len != file.size {
        fs::remove_file(part)?;
        return Err(DownloadError::SizeMismatch {
            file: file.name.clone(),
            expected: file.size,
            actual: len,
        });
    }
    let actual = hex(&hasher.finalize());
    if actual != file.sha256 {
        fs::remove_file(part)?;
        return Err(DownloadError::ChecksumMismatch {
            file: file.name.clone(),
            expected: file.sha256.clone(),
            actual,
        });
    }
    fs::rename(part, dir.join(&file.name))?;
    Ok(())
}

/// Feeds an existing partial file to the hasher and returns its length.
fn hash_into(path: &Path, hasher: &mut Sha256) -> io::Result<u64> {
    let mut file = File::open(path)?;
    let mut buf = vec![0; CHUNK];
    let mut len = 0;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(len);
        }
        hasher.update(&buf[..n]);
        len += n as u64;
    }
}

/// `Content-Range: bytes 200-499/500` → 200.
fn content_range_start(headers: &ureq::http::HeaderMap) -> Option<u64> {
    let value = headers.get("content-range")?.to_str().ok()?;
    let range = value.strip_prefix("bytes ")?;
    range.split('-').next()?.trim().parse().ok()
}

fn part_path(dir: &Path, file: &ModelFile) -> PathBuf {
    dir.join(format!("{}{PART_SUFFIX}", file.name))
}

fn file_len(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .ok()
        .filter(|m| m.is_file())
        .map(|m| m.len())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
