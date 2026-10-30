//! Dictation history (spec §3): a local SQLite file of what was dictated, under a retention
//! policy. Paste last and copy last read from it. Calls block on disk, so make them from a
//! worker thread.
//!
//! Under [`HistoryRetention::Never`] no connection exists and no store is on disk, so nothing can
//! be written. The newest text stays in memory in every mode: the no-text-field fallback and
//! paste last need it for the rest of the session, even when the disk refuses it.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{fs, io};

use rusqlite::{Connection, OptionalExtension, Row, params};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// The default journal mode, not WAL: a WAL file would keep deleted text in a sidecar.
/// `secure_delete` overwrites deleted rows, so expiring an entry erases its text from the file.
const SCHEMA: &str = "
    PRAGMA secure_delete = ON;
    CREATE TABLE IF NOT EXISTS dictations (
        id INTEGER PRIMARY KEY,
        created_at INTEGER NOT NULL,
        raw_text TEXT NOT NULL,
        final_text TEXT NOT NULL,
        app TEXT,
        duration_ms INTEGER NOT NULL,
        vad_ms INTEGER NOT NULL,
        asr_ms INTEGER NOT NULL,
        cleanup_ms INTEGER NOT NULL,
        passes_ms INTEGER NOT NULL,
        insertion_ms INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS dictations_created_at ON dictations (created_at);
    PRAGMA user_version = 1;
";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HistoryRetention {
    #[default]
    Keep,
    /// Entries are deleted once they are 24 hours old.
    Delete24h,
    /// Nothing reaches the disk.
    Never,
}

/// Time spent in each stage of one dictation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Timings {
    pub vad: Duration,
    pub asr: Duration,
    /// Rule cleanup plus LLM cleanup.
    pub cleanup: Duration,
    pub passes: Duration,
    pub insertion: Duration,
}

/// One history entry. Holds user text, so don't log it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dictation {
    pub at: SystemTime,
    pub raw_text: String,
    /// What was inserted. Empty when nothing was, such as a lone "press enter" command.
    pub final_text: String,
    /// The focused app's id.
    pub app: Option<String>,
    /// Length of the recording.
    pub duration: Duration,
    pub timings: Timings,
}

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("history file: {0}")]
    Io(#[from] io::Error),
    #[error("history database: {0}")]
    Database(#[from] rusqlite::Error),
}

#[derive(Debug)]
struct LastText {
    at: SystemTime,
    text: String,
}

/// The app's dictation history. `now` is always passed in, like the dictation state machine's.
#[derive(Debug)]
pub struct History {
    path: PathBuf,
    retention: HistoryRetention,
    connection: Option<Connection>,
    last: Option<LastText>,
}

impl History {
    /// Where the app keeps its history.
    pub fn default_path() -> Option<PathBuf> {
        let dirs = directories::ProjectDirs::from("", "", "wispr-free")?;
        Some(dirs.data_dir().join("history.sqlite3"))
    }

    /// Touches nothing on disk and stores nothing until [`History::set_retention`] says so. Kept
    /// separate so a failing disk at startup leaves a usable history that still has paste last.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            retention: HistoryRetention::Never,
            connection: None,
            last: None,
        }
    }

    /// Applies a policy, at startup and whenever the setting changes. It takes effect on what is
    /// already stored: `Delete24h` prunes, and `Never` erases the store. On error the policy
    /// still holds for later calls.
    pub fn set_retention(
        &mut self,
        retention: HistoryRetention,
        now: SystemTime,
    ) -> Result<(), HistoryError> {
        self.retention = retention;
        if retention == HistoryRetention::Never {
            return self.erase_store();
        }
        self.store()?;
        self.prune(now)
    }

    /// Saves a finished dictation and makes it the last transcript. The last transcript is
    /// updated even if this fails.
    pub fn record(&mut self, dictation: &Dictation) -> Result<(), HistoryError> {
        if !dictation.final_text.is_empty() {
            self.last = Some(LastText {
                at: dictation.at,
                text: dictation.final_text.clone(),
            });
        }
        if self.retention == HistoryRetention::Never {
            return Ok(());
        }
        self.store()?.execute(
            "INSERT INTO dictations (created_at, raw_text, final_text, app, duration_ms, vad_ms,
                 asr_ms, cleanup_ms, passes_ms, insertion_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                unix_ms(dictation.at),
                dictation.raw_text,
                dictation.final_text,
                dictation.app,
                millis(dictation.duration),
                millis(dictation.timings.vad),
                millis(dictation.timings.asr),
                millis(dictation.timings.cleanup),
                millis(dictation.timings.passes),
                millis(dictation.timings.insertion),
            ],
        )?;
        self.prune(dictation.at)
    }

    /// The newest non-empty final text, for paste last and copy last.
    pub fn last_text(&mut self, now: SystemTime) -> Result<Option<String>, HistoryError> {
        let pruned = self.prune(now);
        if let Some(last) = &self.last {
            return Ok(Some(last.text.clone()));
        }
        pruned?;
        if self.retention == HistoryRetention::Never {
            return Ok(None);
        }
        let text = self
            .store()?
            .query_row(
                "SELECT final_text FROM dictations WHERE final_text <> ''
                 ORDER BY created_at DESC, id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(text)
    }

    /// Stored dictations, newest first, for "Recent transcripts".
    pub fn recent(
        &mut self,
        limit: usize,
        now: SystemTime,
    ) -> Result<Vec<Dictation>, HistoryError> {
        if self.retention == HistoryRetention::Never {
            return Ok(Vec::new());
        }
        self.prune(now)?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut statement = self.store()?.prepare(
            "SELECT created_at, raw_text, final_text, app, duration_ms, vad_ms, asr_ms,
                 cleanup_ms, passes_ms, insertion_ms
             FROM dictations ORDER BY created_at DESC, id DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([limit], read_dictation)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Deletes what the policy says has expired. Every read does this too, but the app should
    /// also call it on a timer: an idle app must not sit on expired text.
    pub fn prune(&mut self, now: SystemTime) -> Result<(), HistoryError> {
        if self.retention != HistoryRetention::Delete24h {
            return Ok(());
        }
        let cutoff = unix_ms(now).saturating_sub(DAY_MS);
        if self.last.as_ref().is_some_and(|l| unix_ms(l.at) <= cutoff) {
            self.last = None;
        }
        self.store()?
            .execute("DELETE FROM dictations WHERE created_at <= ?1", [cutoff])?;
        Ok(())
    }

    /// Forgets everything, for "Delete all data". Later dictations are stored as usual.
    pub fn clear(&mut self) -> Result<(), HistoryError> {
        self.last = None;
        self.erase_store()
    }

    fn store(&mut self) -> Result<&Connection, HistoryError> {
        let connection = match self.connection.take() {
            Some(connection) => connection,
            None => open_store(&self.path)?,
        };
        Ok(self.connection.insert(connection))
    }

    fn erase_store(&mut self) -> Result<(), HistoryError> {
        self.connection = None;
        for suffix in ["", "-journal", "-wal", "-shm"] {
            let mut file = self.path.clone().into_os_string();
            file.push(suffix);
            match fs::remove_file(file) {
                Err(error) if error.kind() != ErrorKind::NotFound => return Err(error.into()),
                _ => {}
            }
        }
        Ok(())
    }
}

fn open_store(path: &Path) -> Result<Connection, HistoryError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(path)?;
    restrict_to_owner(path)?;
    connection.execute_batch(SCHEMA)?;
    Ok(connection)
}

/// Linux data folders are often world-readable, and this file holds everything the user said.
#[cfg(unix)]
fn restrict_to_owner(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_to_owner(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn read_dictation(row: &Row) -> rusqlite::Result<Dictation> {
    Ok(Dictation {
        at: UNIX_EPOCH + duration(row.get(0)?),
        raw_text: row.get(1)?,
        final_text: row.get(2)?,
        app: row.get(3)?,
        duration: duration(row.get(4)?),
        timings: Timings {
            vad: duration(row.get(5)?),
            asr: duration(row.get(6)?),
            cleanup: duration(row.get(7)?),
            passes: duration(row.get(8)?),
            insertion: duration(row.get(9)?),
        },
    })
}

fn unix_ms(at: SystemTime) -> i64 {
    at.duration_since(UNIX_EPOCH).map_or(0, millis)
}

fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

fn duration(ms: i64) -> Duration {
    Duration::from_millis(u64::try_from(ms).unwrap_or(0))
}
