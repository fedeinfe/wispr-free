//! Meetings for a check to find in a run's temp home: two small ones made here, or a copy of a real
//! one kept by `real_meeting`. Nothing here reads the user's meetings.

use std::fs;
use std::path::Path;
use std::thread;
use std::time::Duration;

use anyhow::Context as _;
use wf_meetings::{
    ActionItem, Channel, Item, LiveLog, Meeting, MeetingStore, Segment, SpeakerId, Stage,
    StageState, Summary, transcript_hash,
};

pub const FINISHED_TITLE: &str = "Launch review";
pub const UNFINISHED_TITLE: &str = "Budget check";
/// A real meeting's audio has no part in showing it, and it's the one heavy file.
const AUDIO: &str = "ogg";

fn say(log: &mut LiveLog, at_ms: u64, channel: Channel, speaker: Option<u32>, text: &str) {
    let segment = Segment {
        channel,
        speaker: speaker.map(SpeakerId),
        start_ms: at_ms,
        end_ms: at_ms + 2_000,
        text: text.to_owned(),
    };
    log.segment(&segment).expect("a temp folder takes a line");
}

fn finished(store: &MeetingStore) -> anyhow::Result<()> {
    let mut meeting = store.create()?;
    meeting.begin_part()?;
    meeting.end_part(Duration::from_secs(90))?;
    let mut log = LiveLog::open(&meeting)?;
    say(
        &mut log,
        5_000,
        Channel::You,
        None,
        "Can you send the contract?",
    );
    say(
        &mut log,
        12_000,
        Channel::Others,
        Some(1),
        "Yes, by Thursday.",
    );
    say(
        &mut log,
        75_000,
        Channel::Others,
        Some(2),
        "The budget is approved, says finance.",
    );
    drop(log);
    meeting.set_notes("Call the supplier")?;
    let hash = transcript_hash(&meeting.transcript()?);
    meeting.save_summary(&Summary {
        title: Some(FINISHED_TITLE.to_owned()),
        tldr: "The contract goes out by Thursday, and finance approved the budget.".to_owned(),
        key_points: vec![Item {
            text: "The budget is approved".to_owned(),
            at_ms: Some(75_000),
        }],
        decisions: vec![Item {
            text: "A time that is no line".to_owned(),
            at_ms: Some(33_000),
        }],
        action_items: vec![ActionItem {
            task: "Send the contract".to_owned(),
            owner: Some("Speaker 1".to_owned()),
            due: Some("Thursday".to_owned()),
            at_ms: Some(12_000),
        }],
        partial: false,
        transcript_hash: hash,
    })?;
    done(
        &mut meeting,
        &[Stage::Diarization, Stage::Refine, Stage::Summary],
    )
}

fn done(meeting: &mut Meeting, stages: &[Stage]) -> anyhow::Result<()> {
    for stage in stages {
        meeting.set_stage(*stage, StageState::Done { at_ms: 1 })?;
    }
    Ok(())
}

/// One whose speakers failed and whose summary is still to come, with a stage turned off.
fn unfinished(store: &MeetingStore) -> anyhow::Result<()> {
    let mut meeting = store.create()?;
    meeting.set_title(Some(UNFINISHED_TITLE))?;
    meeting.begin_part()?;
    meeting.end_part(Duration::from_secs(600))?;
    let mut log = LiveLog::open(&meeting)?;
    say(&mut log, 5_000, Channel::You, None, "Where do we stand?");
    drop(log);
    let failed = StageState::Failed {
        reason: "the speaker model is damaged".to_owned(),
    };
    meeting.set_stage(Stage::Diarization, failed)?;
    let skipped = StageState::Skipped {
        reason: "turned off in Settings".to_owned(),
    };
    meeting.set_stage(Stage::Refine, skipped)?;
    Ok(())
}

/// Lays out two meetings under `meetings`, the unfinished one last in the list.
pub fn seed(meetings: &Path) -> anyhow::Result<()> {
    let store = MeetingStore::new(meetings);
    unfinished(&store)?;
    // Ids are made from the clock.
    thread::sleep(Duration::from_millis(3));
    finished(&store)
}

/// Copies the meetings under `kept` to `meetings`, leaving their audio behind.
pub fn copy_kept(kept: &Path, meetings: &Path) -> anyhow::Result<()> {
    for meeting in fs::read_dir(kept).with_context(|| kept.display().to_string())? {
        let folder = meeting?.path();
        if !folder.is_dir() {
            continue;
        }
        let name = folder.file_name().context("a meeting's folder")?;
        let copy = meetings.join(name);
        fs::create_dir_all(&copy)?;
        for file in fs::read_dir(&folder)? {
            let file = file?.path();
            if file.is_file() && file.extension().is_none_or(|extension| extension != AUDIO) {
                let name = file.file_name().context("a meeting's file")?;
                fs::copy(&file, copy.join(name))?;
            }
        }
    }
    Ok(())
}
