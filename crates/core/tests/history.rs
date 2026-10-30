//! Dictation history (spec §3): raw and final text, app, duration and timings in a local SQLite
//! file, under a retention policy. Paste last and copy last read from it.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tempfile::TempDir;
use wf_core::history::{Dictation, History, HistoryRetention, Timings};

const SECRET: &str = "the-quick-secret-transcript-7f3a";
const HOUR: Duration = Duration::from_secs(3600);

fn start() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_800_000_000)
}

fn dictation(at: SystemTime, final_text: &str) -> Dictation {
    Dictation {
        at,
        raw_text: format!("um {final_text}"),
        final_text: final_text.to_owned(),
        app: Some("com.apple.TextEdit".to_owned()),
        duration: Duration::from_millis(4_250),
        timings: Timings {
            vad: Duration::from_millis(3),
            asr: Duration::from_millis(410),
            cleanup: Duration::from_millis(220),
            passes: Duration::from_millis(1),
            insertion: Duration::from_millis(520),
        },
    }
}

fn history_at(path: &Path, retention: HistoryRetention) -> History {
    let mut history = History::new(path.to_owned());
    history.set_retention(retention, start()).unwrap();
    history
}

/// Every file under `dir` with its size and modification time.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, (u64, SystemTime)> {
    let mut files = BTreeMap::new();
    let mut pending = vec![dir.to_owned()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(next).unwrap() {
            let entry = entry.unwrap();
            let metadata = entry.metadata().unwrap();
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                files.insert(entry.path(), (metadata.len(), metadata.modified().unwrap()));
            }
        }
    }
    files
}

/// Whether the raw bytes of any file under `dir` contain `needle`.
fn on_disk(dir: &Path, needle: &str) -> bool {
    snapshot(dir).keys().any(|file| {
        let bytes = fs::read(file).unwrap();
        bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
    })
}

#[test]
fn a_kept_dictation_comes_back_whole_after_a_restart() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("data/history.sqlite3");
    let kept = dictation(start(), "So I think we should ship it.");
    history_at(&path, HistoryRetention::Keep)
        .record(&kept)
        .unwrap();

    let mut reopened = history_at(&path, HistoryRetention::Keep);
    assert_eq!(reopened.recent(10, start()).unwrap(), [kept]);
    assert_eq!(
        reopened.last_text(start()).unwrap().as_deref(),
        Some("So I think we should ship it.")
    );
}

#[test]
fn recent_lists_newest_first_up_to_the_limit() {
    let dir = TempDir::new().unwrap();
    let mut history = history_at(&dir.path().join("h.sqlite3"), HistoryRetention::Keep);
    for (i, text) in ["one", "two", "three"].into_iter().enumerate() {
        let at = start() + Duration::from_secs(i as u64);
        history.record(&dictation(at, text)).unwrap();
    }
    let recent = history.recent(2, start() + HOUR).unwrap();
    let texts: Vec<&str> = recent.iter().map(|d| d.final_text.as_str()).collect();
    assert_eq!(texts, ["three", "two"]);
}

#[test]
fn optional_fields_stay_optional() {
    let dir = TempDir::new().unwrap();
    let mut history = history_at(&dir.path().join("h.sqlite3"), HistoryRetention::Keep);
    let no_app = Dictation {
        app: None,
        ..dictation(start(), "text")
    };
    history.record(&no_app).unwrap();
    assert_eq!(history.recent(1, start()).unwrap(), [no_app]);
}

#[test]
fn paste_last_skips_dictations_that_inserted_nothing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("h.sqlite3");
    let mut history = history_at(&path, HistoryRetention::Keep);
    history.record(&dictation(start(), "Real text.")).unwrap();
    history.record(&dictation(start() + HOUR, "")).unwrap();
    assert_eq!(
        history.last_text(start() + HOUR).unwrap().as_deref(),
        Some("Real text.")
    );
    drop(history);

    let mut reopened = history_at(&path, HistoryRetention::Keep);
    assert_eq!(
        reopened.last_text(start() + HOUR).unwrap().as_deref(),
        Some("Real text.")
    );
}

#[test]
fn an_empty_history_has_no_last_text() {
    let dir = TempDir::new().unwrap();
    let mut history = history_at(&dir.path().join("h.sqlite3"), HistoryRetention::Keep);
    assert_eq!(history.last_text(start()).unwrap(), None);
    assert_eq!(history.recent(5, start()).unwrap(), []);
}

#[test]
fn never_store_creates_nothing_on_disk() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("not-yet/history.sqlite3");
    let mut history = history_at(&path, HistoryRetention::Never);

    for i in 0..200 {
        let at = start() + Duration::from_secs(i);
        history.record(&dictation(at, SECRET)).unwrap();
        history.prune(at).unwrap();
        history.last_text(at).unwrap();
        history.recent(5, at).unwrap();
    }
    drop(history);

    assert_eq!(snapshot(dir.path()), BTreeMap::new());
    assert!(!dir.path().join("not-yet").exists());
}

#[test]
fn never_store_still_serves_paste_last_for_this_session_only() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("h.sqlite3");
    let mut history = history_at(&path, HistoryRetention::Never);
    history
        .record(&dictation(start(), "Only in memory."))
        .unwrap();

    assert_eq!(
        history.last_text(start()).unwrap().as_deref(),
        Some("Only in memory.")
    );
    assert_eq!(history.recent(5, start()).unwrap(), []);
    drop(history);

    let mut restarted = history_at(&path, HistoryRetention::Never);
    assert_eq!(restarted.last_text(start()).unwrap(), None);
}

#[test]
fn switching_to_never_store_erases_what_was_kept_and_touches_nothing_else() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("h.sqlite3");
    let neighbour = dir.path().join("settings.toml");
    fs::write(&neighbour, "theme = 'dark'").unwrap();
    let mut history = history_at(&path, HistoryRetention::Keep);
    history.record(&dictation(start(), SECRET)).unwrap();
    assert!(on_disk(dir.path(), SECRET));
    let before_switch = fs::metadata(&neighbour).unwrap().modified().unwrap();

    history
        .set_retention(HistoryRetention::Never, start())
        .unwrap();
    assert!(!on_disk(dir.path(), SECRET));
    assert_eq!(snapshot(dir.path()).len(), 1, "only the neighbour is left");

    let after_erase = snapshot(dir.path());
    for i in 0..50 {
        history
            .record(&dictation(start() + Duration::from_secs(i), SECRET))
            .unwrap();
    }
    assert_eq!(snapshot(dir.path()), after_erase);
    assert_eq!(
        fs::metadata(&neighbour).unwrap().modified().unwrap(),
        before_switch
    );
}

#[test]
fn starting_with_never_store_erases_a_store_left_by_an_earlier_run() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("h.sqlite3");
    history_at(&path, HistoryRetention::Keep)
        .record(&dictation(start(), SECRET))
        .unwrap();
    fs::write(dir.path().join("h.sqlite3-journal"), SECRET).unwrap();
    fs::write(dir.path().join("h.sqlite3-wal"), SECRET).unwrap();

    let _history = history_at(&path, HistoryRetention::Never);
    assert_eq!(snapshot(dir.path()), BTreeMap::new());
}

#[test]
fn a_history_that_was_never_given_a_retention_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("h.sqlite3");
    let mut history = History::new(path);
    history.record(&dictation(start(), SECRET)).unwrap();
    assert_eq!(snapshot(dir.path()), BTreeMap::new());
}

#[test]
fn turning_storing_back_on_keeps_only_what_comes_next() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("h.sqlite3");
    let mut history = history_at(&path, HistoryRetention::Never);
    history.record(&dictation(start(), SECRET)).unwrap();
    history
        .set_retention(HistoryRetention::Keep, start())
        .unwrap();
    assert!(!on_disk(dir.path(), SECRET));

    history
        .record(&dictation(start() + HOUR, "Stored now."))
        .unwrap();
    let texts: Vec<String> = history
        .recent(5, start() + HOUR)
        .unwrap()
        .into_iter()
        .map(|d| d.final_text)
        .collect();
    assert_eq!(texts, ["Stored now."]);
}

#[test]
fn delete_after_24_hours_erases_old_entries_from_disk() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("h.sqlite3");
    let mut history = history_at(&path, HistoryRetention::Delete24h);
    history.record(&dictation(start(), SECRET)).unwrap();
    history
        .record(&dictation(start() + HOUR, "Newer."))
        .unwrap();

    let just_before = start() + 24 * HOUR - Duration::from_secs(1);
    assert_eq!(history.recent(5, just_before).unwrap().len(), 2);

    history.prune(start() + 24 * HOUR).unwrap();
    let left = history.recent(5, start() + 24 * HOUR).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].final_text, "Newer.");
    assert!(
        !on_disk(dir.path(), SECRET),
        "deleted text must not linger in the file"
    );
}

#[test]
fn reads_never_return_an_expired_entry_even_without_a_prune() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("h.sqlite3");
    let mut history = history_at(&path, HistoryRetention::Delete24h);
    history.record(&dictation(start(), "Old.")).unwrap();

    assert_eq!(history.last_text(start() + 25 * HOUR).unwrap(), None);
    assert_eq!(history.recent(5, start() + 25 * HOUR).unwrap(), []);
}

#[test]
fn an_expired_entry_is_gone_after_a_restart_too() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("h.sqlite3");
    history_at(&path, HistoryRetention::Delete24h)
        .record(&dictation(start(), SECRET))
        .unwrap();

    let mut restarted = History::new(path.clone());
    restarted
        .set_retention(HistoryRetention::Delete24h, start() + 25 * HOUR)
        .unwrap();
    assert!(!on_disk(dir.path(), SECRET));
}

#[test]
fn switching_to_24_hours_applies_to_entries_already_kept() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("h.sqlite3");
    let mut history = history_at(&path, HistoryRetention::Keep);
    history.record(&dictation(start(), SECRET)).unwrap();
    assert_eq!(history.recent(5, start() + 48 * HOUR).unwrap().len(), 1);

    history
        .set_retention(HistoryRetention::Delete24h, start() + 48 * HOUR)
        .unwrap();
    assert_eq!(history.recent(5, start() + 48 * HOUR).unwrap(), []);
    assert!(!on_disk(dir.path(), SECRET));
}

#[test]
fn clear_forgets_everything_and_recording_still_works_afterwards() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("h.sqlite3");
    let mut history = history_at(&path, HistoryRetention::Keep);
    history.record(&dictation(start(), SECRET)).unwrap();

    history.clear().unwrap();
    assert_eq!(history.last_text(start()).unwrap(), None);
    assert!(!on_disk(dir.path(), SECRET));

    history.record(&dictation(start(), "Again.")).unwrap();
    assert_eq!(history.recent(5, start()).unwrap().len(), 1);
}

#[test]
fn paste_last_survives_a_store_that_cannot_be_written() {
    let dir = TempDir::new().unwrap();
    let mut history = History::new(dir.path().to_owned()); // a directory is no database
    assert!(
        history
            .set_retention(HistoryRetention::Keep, start())
            .is_err()
    );

    assert!(history.record(&dictation(start(), "Still here.")).is_err());
    assert_eq!(
        history.last_text(start()).unwrap().as_deref(),
        Some("Still here.")
    );
}

#[cfg(unix)]
#[test]
fn only_the_owner_can_read_the_store() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("h.sqlite3");
    let mut history = history_at(&path, HistoryRetention::Keep);
    history.record(&dictation(start(), "Private.")).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn history_can_move_to_a_worker_thread() {
    fn assert_send<T: Send>() {}
    assert_send::<History>();
}
