//! The list, a meeting in full, search and export, and what older builds left on disk.

use std::fs;
use std::path::Path;
use std::thread;
use std::time::Duration;

use tempfile::TempDir;
use wf_core::audio::FRAME_SAMPLES;
use wf_meetings::{
    ActionItem, Channel, Item, LiveLog, Meeting, MeetingStore, MeetingWriter, SearchIndex, Segment,
    SpeakerId, Stage, StageState, Summary, export_file_name, export_markdown, meeting_detail,
    meeting_entries, write_export,
};

fn store() -> (TempDir, MeetingStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::new(dir.path());
    (dir, store)
}

fn say(meeting: &Meeting, lines: &[(u64, Option<u32>, &str)]) {
    let mut log = LiveLog::open(meeting).unwrap();
    for &(start_ms, speaker, text) in lines {
        log.segment(&Segment {
            channel: if speaker.is_some() {
                Channel::Others
            } else {
                Channel::You
            },
            speaker: speaker.map(SpeakerId),
            start_ms,
            end_ms: start_ms + 3_000,
            text: text.to_owned(),
        })
        .unwrap();
    }
}

fn summary() -> Summary {
    Summary {
        title: Some("Pricing review".into()),
        tldr: "Speaker 1 takes the pricing page.".into(),
        key_points: vec![Item {
            text: "Pricing page needs new copy".into(),
            at_ms: Some(4_000),
        }],
        decisions: vec![Item {
            text: "Free tier stays at five projects".into(),
            at_ms: Some(9_000),
        }],
        action_items: vec![ActionItem {
            task: "update the pricing page".into(),
            owner: Some("Speaker 1".into()),
            due: Some("next week".into()),
            at_ms: Some(12_000),
        }],
        partial: false,
        transcript_hash: String::new(),
    }
}

fn tone() -> Vec<f32> {
    (0..FRAME_SAMPLES)
        .map(|n| 0.3 * (std::f32::consts::TAU * n as f32 / 40.0).sin())
        .collect()
}

fn write_audio(path: &Path, seconds: usize) -> Duration {
    let mut writer = MeetingWriter::create(path).unwrap();
    for _ in 0..seconds * 25 {
        writer.write_frame(&tone(), &tone()).unwrap();
    }
    writer.finish().unwrap()
}

/// A recording cut off by a crash: pages on disk, and no end of stream.
fn crashed_audio(path: &Path) {
    let mut writer = MeetingWriter::create(path).unwrap();
    for _ in 0..80 {
        writer.write_frame(&tone(), &tone()).unwrap();
    }
    std::mem::forget(writer);
}

#[test]
fn the_list_shows_the_users_title_first_then_the_summarys() {
    let (_dir, store) = store();
    let mut named = store.create().unwrap();
    named.set_title(Some("Mine")).unwrap();
    named.save_summary(&summary()).unwrap();
    let mut summarized = store.create().unwrap();
    summarized.save_summary(&summary()).unwrap();
    store.create().unwrap();

    let entries = meeting_entries(&store).unwrap();

    let titles: Vec<_> = entries.iter().map(|e| e.title.as_deref()).collect();
    assert_eq!(titles, [None, Some("Pricing review"), Some("Mine")]);
}

#[test]
fn the_list_adds_up_the_parts_of_a_resumed_meeting() {
    let (_dir, store) = store();
    let mut meeting = store.create().unwrap();
    for seconds in [2, 3] {
        let part = meeting.begin_part().unwrap();
        let length = write_audio(&part.path, seconds);
        meeting.end_part(length).unwrap();
    }

    let entries = meeting_entries(&store).unwrap();

    assert!(
        (4_900..=5_100).contains(&entries[0].length_ms),
        "{entries:?}"
    );
}

#[test]
fn a_failed_stage_shows_in_the_list() {
    let (_dir, store) = store();
    let mut meeting = store.create().unwrap();
    meeting
        .set_stage(
            Stage::Summary,
            StageState::Failed {
                reason: "no model".into(),
            },
        )
        .unwrap();

    assert!(meeting_entries(&store).unwrap()[0].has_failure());
}

#[test]
fn a_meeting_in_full_has_its_names_notes_and_voices() {
    let (_dir, store) = store();
    let mut meeting = store.create().unwrap();
    say(
        &meeting,
        &[
            (1_000, None, "Hello."),
            (5_000, Some(2), "Hi."),
            (9_000, Some(1), "Hey."),
        ],
    );
    meeting.rename_speaker(SpeakerId(2), "Giulia").unwrap();
    meeting.set_notes("Bring the slides").unwrap();
    let id = meeting.record().id.clone();

    let detail = meeting_detail(&store, &id).unwrap();

    assert_eq!(detail.notes, "Bring the slides");
    assert_eq!(detail.rows[1].name, "Giulia");
    assert_eq!(
        detail.speakers,
        [
            (SpeakerId(1), None),
            (SpeakerId(2), Some("Giulia".to_owned()))
        ]
    );
}

#[test]
fn a_meeting_whose_stages_never_ran_is_unfinished() {
    let (_dir, store) = store();
    let mut meeting = store.create().unwrap();
    let part = meeting.begin_part().unwrap();
    let id = meeting.record().id.clone();
    assert!(
        !meeting_detail(&store, &id).unwrap().unfinished,
        "still recording"
    );

    let length = write_audio(&part.path, 1);
    meeting.end_part(length).unwrap();
    assert!(meeting_detail(&store, &id).unwrap().unfinished);

    for stage in [Stage::Diarization, Stage::Refine, Stage::Summary] {
        meeting.set_stage(stage, StageState::done()).unwrap();
    }
    assert!(!meeting_detail(&store, &id).unwrap().unfinished);
}

#[test]
fn search_looks_in_titles_summaries_notes_and_transcripts() {
    let (_dir, store) = store();
    let mut titled = store.create().unwrap();
    titled.set_title(Some("Quarterly budget")).unwrap();
    let mut summarized = store.create().unwrap();
    summarized.save_summary(&summary()).unwrap();
    let mut noted = store.create().unwrap();
    noted.set_notes("Call the landlord tomorrow").unwrap();
    let spoken = store.create().unwrap();
    say(
        &spoken,
        &[(61_000, Some(1), "The warehouse lease ends in May.")],
    );

    let index = SearchIndex::build(&store).unwrap();
    let id = |hits: Vec<wf_meetings::SearchHit>| hits[0].id.clone();

    assert_eq!(id(index.search("BUDGET")), titled.record().id);
    assert_eq!(id(index.search("five projects")), summarized.record().id);
    assert_eq!(id(index.search("landlord")), noted.record().id);
    let hits = index.search("warehouse");
    assert_eq!(hits[0].id, spoken.record().id);
    assert_eq!(hits[0].row, Some(0));
    assert!(
        hits[0].snippet.starts_with("[01:01] "),
        "{}",
        hits[0].snippet
    );
}

#[test]
fn search_needs_every_word_but_not_in_one_place() {
    let (_dir, store) = store();
    let mut meeting = store.create().unwrap();
    meeting.set_title(Some("Budget")).unwrap();
    say(&meeting, &[(1_000, None, "The landlord called.")]);
    let index = SearchIndex::build(&store).unwrap();

    assert_eq!(index.search("budget landlord").len(), 1);
    assert!(index.search("budget giraffe").is_empty());
    assert!(index.search("   ").is_empty());
}

#[test]
fn search_ignores_accents_and_case() {
    let (_dir, store) = store();
    let mut meeting = store.create().unwrap();
    meeting.set_notes("Città di Forlì").unwrap();
    let index = SearchIndex::build(&store).unwrap();

    assert_eq!(index.search("citta forli").len(), 1);
    assert!(index.search("FORLÌ")[0].snippet.contains("Forlì"));
}

#[test]
fn search_finds_a_speakers_new_name() {
    let (_dir, store) = store();
    let mut meeting = store.create().unwrap();
    say(&meeting, &[(1_000, Some(1), "Hello.")]);
    meeting.rename_speaker(SpeakerId(1), "Giulia").unwrap();

    let index = SearchIndex::build(&store).unwrap();

    assert_eq!(index.search("giulia").len(), 1);
}

#[test]
fn an_export_has_every_part_with_names_applied() {
    let (_dir, store) = store();
    let mut meeting = store.create().unwrap();
    say(
        &meeting,
        &[
            (4_000, None, "The pricing page needs work."),
            (12_000, Some(1), "I will do it."),
        ],
    );
    meeting.save_summary(&summary()).unwrap();
    meeting.rename_speaker(SpeakerId(1), "Giulia").unwrap();
    meeting.set_notes("Check the VAT note").unwrap();
    let detail = meeting_detail(&store, &meeting.record().id).unwrap();

    let text = export_markdown(&detail, "30 October 2026");

    assert!(text.starts_with("# Pricing review\n\n30 October 2026"));
    assert!(text.contains("Giulia takes the pricing page."));
    assert!(text.contains("- [00:09] Free tier stays at five projects"));
    assert!(text.contains("- [00:12] Giulia: update the pricing page (next week)"));
    assert!(text.contains("## Notes\n\nCheck the VAT note"));
    assert!(text.contains("[00:12] Giulia: I will do it."));
    assert!(text.contains("can be wrong"));
}

#[test]
fn an_export_with_no_summary_or_notes_leaves_those_sections_out() {
    let (_dir, store) = store();
    let meeting = store.create().unwrap();
    say(&meeting, &[(4_000, None, "Hello.")]);
    let detail = meeting_detail(&store, &meeting.record().id).unwrap();

    let text = export_markdown(&detail, "today");

    assert!(text.starts_with("# Meeting\n"));
    assert!(!text.contains("## Summary"));
    assert!(!text.contains("## Notes"));
    assert!(text.contains("## Transcript"));
}

#[test]
fn an_export_never_writes_over_another_file() {
    let (dir, store) = store();
    let mut meeting = store.create().unwrap();
    meeting.set_title(Some("Plan: Q3/Q4?")).unwrap();
    let detail = meeting_detail(&store, &meeting.record().id).unwrap();
    let folder = dir.path().join("exports");

    let first = write_export(&folder, &detail, "today").unwrap();
    let second = write_export(&folder, &detail, "today").unwrap();

    assert_eq!(first.file_name().unwrap(), "Plan Q3 Q4.md");
    assert_eq!(second.file_name().unwrap(), "Plan Q3 Q4 2.md");
    assert!(first.exists());
}

#[test]
fn a_file_name_takes_nothing_a_file_system_refuses() {
    assert_eq!(export_file_name(None), "Meeting");
    assert_eq!(export_file_name(Some("...")), "Meeting");
    assert_eq!(export_file_name(Some("a/b\\c:d")), "a b c d");
    assert_eq!(export_file_name(Some(&"x".repeat(200))).len(), 60);
}

#[test]
fn a_rename_from_the_window_survives_a_stage_written_from_an_older_copy() {
    let (_dir, store) = store();
    let mut worker = store.create().unwrap();
    let mut window = store.open(&worker.record().id).unwrap();

    window.set_title(Some("Mine")).unwrap();
    window.rename_speaker(SpeakerId(1), "Giulia").unwrap();
    worker.set_stage(Stage::Refine, StageState::done()).unwrap();

    let saved = store.open(&worker.record().id).unwrap();
    assert_eq!(saved.record().title.as_deref(), Some("Mine"));
    assert_eq!(saved.record().speakers[&SpeakerId(1)], "Giulia");
    assert!(saved.record().stages.refine.is_done());
}

#[test]
fn many_threads_changing_a_record_lose_nothing() {
    let (_dir, store) = store();
    let id = store.create().unwrap().record().id.clone();
    let threads: Vec<_> = (0..8u32)
        .map(|n| {
            let (store, id) = (store.clone(), id.clone());
            thread::spawn(move || {
                let mut meeting = store.open(&id).unwrap();
                meeting.rename_speaker(SpeakerId(n), "name").unwrap();
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }

    assert_eq!(store.open(&id).unwrap().record().speakers.len(), 8);
}

#[test]
fn a_finished_flat_recording_becomes_a_meeting_with_its_audio() {
    let (dir, store) = store();
    let flat = dir.path().join("1760000000.ogg");
    write_audio(&flat, 2);

    let adopted = store.adopt_flat_recordings().unwrap();

    assert_eq!(adopted.len(), 1);
    assert!(!flat.exists());
    let meeting = store.open(&adopted[0]).unwrap();
    assert_eq!(meeting.record().created_ms, 1_760_000_000_000);
    assert!(!meeting.is_recording());
    assert!(meeting.audio().unwrap().you.duration() >= Duration::from_millis(1_900));
    assert!(meeting_detail(&store, &adopted[0]).unwrap().unfinished);
}

#[test]
fn a_flat_recording_a_crash_left_is_repaired_on_the_way_in() {
    let (dir, store) = store();
    crashed_audio(&dir.path().join("1760000000.recording"));

    let adopted = store.adopt_flat_recordings().unwrap();

    let meeting = store.open(&adopted[0]).unwrap();
    assert!(meeting.audio().unwrap().you.duration() >= Duration::from_secs(1));
    assert!(!meeting.is_recording());
}

#[test]
fn files_that_are_not_flat_recordings_stay_put() {
    let (dir, store) = store();
    let junk = dir.path().join("1760000000.recording");
    fs::write(&junk, "not a meeting").unwrap();
    let other = dir.path().join("notes.ogg");
    fs::write(&other, "named by someone else").unwrap();

    let adopted = store.adopt_flat_recordings().unwrap();

    assert!(adopted.is_empty());
    assert!(junk.exists() && other.exists());
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn adopting_twice_changes_nothing() {
    let (dir, store) = store();
    write_audio(&dir.path().join("1760000000.ogg"), 1);

    store.adopt_flat_recordings().unwrap();

    assert!(store.adopt_flat_recordings().unwrap().is_empty());
    assert_eq!(store.list().unwrap().len(), 1);
}

#[test]
fn a_crash_before_the_recording_wrote_anything_leaves_a_meeting_with_no_part() {
    let (_dir, store) = store();
    let mut meeting = store.create().unwrap();
    meeting.begin_part().unwrap();

    let recovered = store.recover().unwrap();

    assert_eq!(recovered, [meeting.record().id.clone()]);
    let meeting = store.open(&meeting.record().id).unwrap();
    assert!(!meeting.is_recording());
    assert!(meeting.record().parts.is_empty());
}

#[test]
fn a_damaged_recording_is_set_aside_and_does_not_stop_the_others() {
    let (dir, store) = store();
    let mut damaged = store.create().unwrap();
    let part = damaged.begin_part().unwrap();
    fs::write(&part.path, "not ogg").unwrap();
    let mut good = store.create().unwrap();
    let part = good.begin_part().unwrap();
    crashed_audio(&part.path);

    let recovered = store.recover().unwrap();

    assert_eq!(recovered.len(), 2);
    let saved = store.open(&good.record().id).unwrap();
    assert!(!saved.is_recording());
    let folder = dir.path().join(&damaged.record().id);
    assert!(folder.join("part-000.ogg.bad").exists());
}

#[test]
fn a_recording_that_never_started_can_be_taken_back() {
    let (_dir, store) = store();
    let mut meeting = store.create().unwrap();
    meeting.begin_part().unwrap();

    meeting.abandon_part().unwrap();

    assert!(!meeting.is_recording());
    assert!(meeting.record().parts.is_empty());
}
