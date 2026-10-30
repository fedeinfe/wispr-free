//! The on-disk meeting record: one folder per meeting, each stage in its own file.

use std::fs;
use std::path::Path;
use std::time::Duration;

use tempfile::TempDir;
use wf_core::audio::FRAME_SAMPLES;
use wf_meetings::{
    Channel, Error, Gap, GapReason, LiveLog, Meeting, MeetingStore, MeetingWriter,
    RefinedTranscript, Segment, SpeakerId, StageState, Transcript, parse_timestamp, timestamp,
};

fn store() -> (TempDir, MeetingStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::new(dir.path());
    (dir, store)
}

fn segment(channel: Channel, start_ms: u64, end_ms: u64, text: &str) -> Segment {
    Segment {
        channel,
        speaker: None,
        start_ms,
        end_ms,
        text: text.to_owned(),
    }
}

fn gap(channel: Channel, start_ms: u64, end_ms: u64) -> Gap {
    Gap {
        channel,
        start_ms,
        end_ms,
        reason: GapReason::Behind,
    }
}

/// Records one second of tone to `path`.
fn write_audio(path: &Path, seconds: usize) -> Duration {
    let mut writer = MeetingWriter::create(path).unwrap();
    let tone: Vec<f32> = (0..FRAME_SAMPLES)
        .map(|n| 0.3 * (std::f32::consts::TAU * n as f32 / 40.0).sin())
        .collect();
    for _ in 0..seconds * 25 {
        writer.write_frame(&tone, &tone).unwrap();
    }
    writer.finish().unwrap()
}

fn record_part(meeting: &mut Meeting, seconds: usize) -> Duration {
    let part = meeting.begin_part().unwrap();
    let length = write_audio(&part.path, seconds);
    meeting.end_part(length).unwrap();
    length
}

#[test]
fn a_new_meeting_is_listed_and_opens_again() {
    let (_dir, store) = store();

    let meeting = store.create().unwrap();
    let id = meeting.record().id.clone();

    let listed = store.list().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, id);
    assert_eq!(store.open(&id).unwrap().record(), meeting.record());
}

#[test]
fn the_newest_meeting_is_listed_first() {
    let (_dir, store) = store();
    let older = store.create().unwrap();
    let newer = store.create().unwrap();

    let ids: Vec<String> = store.list().unwrap().into_iter().map(|m| m.id).collect();

    assert_eq!(ids, [newer.record().id.clone(), older.record().id.clone()]);
}

#[test]
fn a_folder_that_is_not_a_meeting_does_not_break_the_list() {
    let (dir, store) = store();
    store.create().unwrap();
    fs::create_dir(dir.path().join("not-a-meeting")).unwrap();
    fs::write(dir.path().join("stray.txt"), "hello").unwrap();

    assert_eq!(store.list().unwrap().len(), 1);
}

#[test]
fn a_listed_id_cannot_reach_outside_the_store() {
    let (_dir, store) = store();

    assert!(matches!(
        store.open("../elsewhere"),
        Err(Error::NoSuchMeeting(_))
    ));
    assert!(matches!(
        store.delete("../elsewhere"),
        Err(Error::NoSuchMeeting(_))
    ));
}

#[test]
fn deleting_removes_the_whole_folder() {
    let (dir, store) = store();
    let mut meeting = store.create().unwrap();
    record_part(&mut meeting, 1);
    let id = meeting.record().id.clone();

    store.delete(&id).unwrap();

    assert!(!dir.path().join(&id).exists());
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn a_record_from_a_newer_build_is_refused_rather_than_rewritten() {
    let (dir, store) = store();
    let meeting = store.create().unwrap();
    let id = meeting.record().id.clone();
    let path = dir.path().join(&id).join("meeting.json");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replacen("\"schema\": 1", "\"schema\": 99", 1)).unwrap();

    assert!(matches!(store.open(&id), Err(Error::NewerRecord(_))));
}

#[test]
fn the_title_notes_and_speaker_names_survive_reopening() {
    let (_dir, store) = store();
    let mut meeting = store.create().unwrap();
    meeting.set_title(Some("Pricing review")).unwrap();
    meeting.set_notes("Ask about Q4").unwrap();
    meeting.rename_speaker(SpeakerId(2), "Giulia").unwrap();

    let reopened = store.open(&meeting.record().id).unwrap();

    assert_eq!(reopened.record().title.as_deref(), Some("Pricing review"));
    assert_eq!(reopened.notes().unwrap(), "Ask about Q4");
    assert_eq!(reopened.record().speakers[&SpeakerId(2)], "Giulia");
}

#[test]
fn renaming_to_nothing_restores_the_default_label() {
    let (_dir, store) = store();
    let mut meeting = store.create().unwrap();
    meeting.rename_speaker(SpeakerId(2), "Giulia").unwrap();

    meeting.rename_speaker(SpeakerId(2), "  ").unwrap();

    assert!(meeting.record().speakers.is_empty());
}

#[test]
fn the_live_transcript_comes_back_in_time_order() {
    let (_dir, store) = store();
    let meeting = store.create().unwrap();
    let mut log = LiveLog::open(&meeting).unwrap();

    log.segment(&segment(Channel::Others, 5_000, 6_000, "second"))
        .unwrap();
    log.segment(&segment(Channel::You, 1_000, 2_000, "first"))
        .unwrap();
    log.gap(&gap(Channel::You, 8_000, 9_000)).unwrap();

    let transcript = meeting.transcript().unwrap();
    let texts: Vec<&str> = transcript
        .segments
        .iter()
        .map(|s| s.text.as_str())
        .collect();
    assert_eq!(texts, ["first", "second"]);
    assert_eq!(transcript.gaps, [gap(Channel::You, 8_000, 9_000)]);
}

/// A crash can cut the last line short. It must not cost the lines before it, or the next one.
#[test]
fn a_torn_live_line_loses_only_itself() {
    let (dir, store) = store();
    let meeting = store.create().unwrap();
    let path = dir.path().join(&meeting.record().id).join("live.jsonl");
    let mut log = LiveLog::open(&meeting).unwrap();
    log.segment(&segment(Channel::You, 0, 1_000, "kept"))
        .unwrap();
    drop(log);
    let mut bytes = fs::read(&path).unwrap();
    bytes.extend_from_slice(b"{\"segment\":{\"chan");
    fs::write(&path, bytes).unwrap();

    let mut log = LiveLog::open(&meeting).unwrap();
    log.segment(&segment(Channel::You, 2_000, 3_000, "after the crash"))
        .unwrap();

    let texts: Vec<String> = meeting
        .transcript()
        .unwrap()
        .segments
        .into_iter()
        .map(|s| s.text)
        .collect();
    assert_eq!(texts, ["kept", "after the crash"]);
}

#[test]
fn the_refined_transcript_replaces_the_live_one() {
    let (_dir, store) = store();
    let mut meeting = store.create().unwrap();
    let mut log = LiveLog::open(&meeting).unwrap();
    log.segment(&segment(Channel::You, 0, 1_000, "live words"))
        .unwrap();
    log.gap(&gap(Channel::You, 1_000, 2_000)).unwrap();

    meeting
        .save_refined(&RefinedTranscript {
            covers_ms: 5_000,
            transcript: Transcript {
                segments: vec![segment(Channel::You, 0, 1_000, "better words")],
                gaps: Vec::new(),
            },
        })
        .unwrap();

    let transcript = meeting.transcript().unwrap();
    assert_eq!(transcript.segments.len(), 1);
    assert_eq!(transcript.segments[0].text, "better words");
    assert!(transcript.gaps.is_empty());
}

/// After a resumed recording the new part only has live text until refine runs again.
#[test]
fn live_text_past_the_refined_part_is_kept() {
    let (_dir, store) = store();
    let mut meeting = store.create().unwrap();
    let mut log = LiveLog::open(&meeting).unwrap();
    log.segment(&segment(Channel::You, 0, 1_000, "old live"))
        .unwrap();
    log.segment(&segment(Channel::You, 6_000, 7_000, "resumed"))
        .unwrap();
    meeting
        .save_refined(&RefinedTranscript {
            covers_ms: 5_000,
            transcript: Transcript {
                segments: vec![segment(Channel::You, 0, 1_000, "refined")],
                gaps: Vec::new(),
            },
        })
        .unwrap();

    let texts: Vec<String> = meeting
        .transcript()
        .unwrap()
        .segments
        .into_iter()
        .map(|s| s.text)
        .collect();
    assert_eq!(texts, ["refined", "resumed"]);
}

#[test]
fn a_resumed_recording_continues_the_timeline_in_a_new_file() {
    let (_dir, store) = store();
    let mut meeting = store.create().unwrap();
    let first = record_part(&mut meeting, 2);

    let part = meeting.begin_part().unwrap();

    assert_eq!(part.offset, first);
    assert!(!part.path.exists());
    let second = write_audio(&part.path, 3);
    meeting.end_part(second).unwrap();
    assert_eq!(meeting.length(), first + second);

    let audio = meeting.audio().unwrap();
    assert_eq!(audio.you.duration(), first + second);
    assert_eq!(audio.others.duration(), first + second);
}

#[test]
fn only_one_part_records_at_a_time() {
    let (_dir, store) = store();
    let mut meeting = store.create().unwrap();
    meeting.begin_part().unwrap();

    assert!(matches!(meeting.begin_part(), Err(Error::StillRecording)));
}

#[test]
fn resuming_reopens_the_stages_the_new_audio_makes_stale() {
    let (_dir, store) = store();
    let mut meeting = store.create().unwrap();
    record_part(&mut meeting, 1);
    meeting
        .set_stage(wf_meetings::Stage::Refine, StageState::done())
        .unwrap();

    meeting.begin_part().unwrap();

    assert_eq!(meeting.record().stages.refine, StageState::Pending);
}

/// The crash left a recording with no end marker, and the record still says it is running.
#[test]
fn recovery_finishes_an_interrupted_recording() {
    let (_dir, store) = store();
    let mut meeting = store.create().unwrap();
    let part = meeting.begin_part().unwrap();
    {
        let mut writer = MeetingWriter::create(&part.path).unwrap();
        let tone = vec![0.1; FRAME_SAMPLES];
        for _ in 0..100 {
            writer.write_frame(&tone, &tone).unwrap();
        }
        std::mem::forget(writer);
    }
    let mut log = LiveLog::open(&meeting).unwrap();
    log.segment(&segment(Channel::You, 0, 1_000, "before the crash"))
        .unwrap();
    let id = meeting.record().id.clone();
    drop(meeting);

    let recovered = store.recover().unwrap();

    assert_eq!(recovered, [id.clone()]);
    let meeting = store.open(&id).unwrap();
    assert!(!meeting.is_recording());
    assert!(meeting.length() >= Duration::from_secs(3));
    let transcript = meeting.transcript().unwrap();
    assert_eq!(transcript.segments.len(), 1);
    assert!(
        transcript
            .gaps
            .iter()
            .any(|g| g.reason == GapReason::Interrupted && g.start_ms == 1_000),
        "{:?}",
        transcript.gaps
    );
    assert!(store.recover().unwrap().is_empty());
}

#[test]
fn audio_goes_only_once_the_transcript_is_final() {
    let (dir, store) = store();
    let mut meeting = store.create().unwrap();
    record_part(&mut meeting, 1);
    let mut log = LiveLog::open(&meeting).unwrap();
    log.gap(&gap(Channel::You, 0, 500)).unwrap();
    assert!(matches!(
        meeting.delete_audio(),
        Err(Error::TranscriptNotFinal)
    ));
    meeting
        .set_stage(wf_meetings::Stage::Refine, StageState::done())
        .unwrap();
    meeting
        .set_stage(
            wf_meetings::Stage::Diarization,
            StageState::Skipped {
                reason: "off".to_owned(),
            },
        )
        .unwrap();
    assert!(matches!(
        meeting.delete_audio(),
        Err(Error::TranscriptNotFinal)
    ));

    meeting
        .save_refined(&RefinedTranscript {
            covers_ms: 1_000,
            transcript: Transcript::default(),
        })
        .unwrap();
    meeting.delete_audio().unwrap();

    let folder = dir.path().join(&meeting.record().id);
    assert!(!folder.join("part-000.ogg").exists());
    assert!(meeting.record().audio_deleted);
    assert!(matches!(meeting.audio(), Err(Error::AudioDeleted)));
    assert_eq!(meeting.length(), Duration::from_secs(1));
}

#[test]
fn timestamps_read_as_minutes_and_seconds_and_parse_back() {
    assert_eq!(timestamp(0), "[00:00]");
    assert_eq!(timestamp(65_999), "[01:05]");
    assert_eq!(timestamp(95 * 60_000), "[95:00]");

    assert_eq!(parse_timestamp("[01:05]"), Some(65_000));
    assert_eq!(parse_timestamp("[95:00]"), Some(5_700_000));
    assert_eq!(parse_timestamp("[1:5]"), None);
    assert_eq!(parse_timestamp("[01:75]"), None);
    assert_eq!(parse_timestamp("01:05"), None);
}

#[test]
fn transcript_lines_use_the_new_names_and_the_stable_labels_otherwise() {
    let (_dir, store) = store();
    let mut meeting = store.create().unwrap();
    meeting.rename_speaker(SpeakerId(1), "Giulia").unwrap();
    let transcript = Transcript {
        segments: vec![
            segment(Channel::You, 1_000, 2_000, "hello"),
            Segment {
                speaker: Some(SpeakerId(1)),
                ..segment(Channel::Others, 3_000, 4_000, "hi")
            },
            Segment {
                speaker: Some(SpeakerId(2)),
                ..segment(Channel::Others, 5_000, 6_000, "hey")
            },
            segment(Channel::Others, 7_000, 8_000, "unknown"),
        ],
        gaps: vec![gap(Channel::Others, 9_000, 12_000)],
    };

    assert_eq!(
        transcript.lines(&meeting.record().speakers),
        [
            "[00:01] You: hello",
            "[00:03] Giulia: hi",
            "[00:05] Speaker 2: hey",
            "[00:07] Others: unknown",
            "[00:09] Others: (not transcribed, 3 s)",
        ]
    );
}
