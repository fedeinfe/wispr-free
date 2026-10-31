//! The meetings a check finds in its temp home.

use std::fs;

use tempfile::tempdir;
use wf_meetings::{MeetingStore, meeting_detail, meeting_entries};
use xtask::meeting_seed::{FINISHED_TITLE, UNFINISHED_TITLE, copy_kept, seed};

#[test]
fn two_meetings_come_out_newest_first_with_what_a_walk_presses() {
    let folder = tempdir().unwrap();
    seed(folder.path()).unwrap();

    let store = MeetingStore::new(folder.path());
    let entries = meeting_entries(&store).unwrap();
    let titles: Vec<_> = entries.iter().map(|entry| entry.title.as_deref()).collect();
    assert_eq!(titles, [Some(FINISHED_TITLE), Some(UNFINISHED_TITLE)]);

    let finished = meeting_detail(&store, &entries[0].id).unwrap();
    assert_eq!(finished.rows.len(), 3);
    assert!(finished.summary.is_some());
    assert!(!finished.unfinished);
    let unfinished = meeting_detail(&store, &entries[1].id).unwrap();
    assert!(unfinished.summary.is_none());
    assert!(entries[1].has_failure());
}

#[test]
fn a_copy_of_a_kept_meeting_leaves_its_audio_behind() {
    let (kept, meetings) = (tempdir().unwrap(), tempdir().unwrap());
    let meeting = kept.path().join("0001");
    fs::create_dir_all(&meeting).unwrap();
    fs::write(meeting.join("meeting.json"), "{}").unwrap();
    fs::write(meeting.join("part-000.ogg"), "audio").unwrap();

    copy_kept(kept.path(), meetings.path()).unwrap();

    assert!(meetings.path().join("0001/meeting.json").is_file());
    assert!(!meetings.path().join("0001/part-000.ogg").exists());
}
