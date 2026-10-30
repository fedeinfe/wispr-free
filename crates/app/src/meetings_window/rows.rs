//! What the window shows, worked out from the meeting data. No window here, so all of it can be
//! tested as text.

use chrono::{Local, TimeZone};
use slint::{ModelRc, VecModel};
use wf_dictation::{MeetingsStatus, WorkStep};
use wf_meetings::{
    ChatTurn, MeetingDetail, MeetingEntry, SearchHit, Stage, StageState, Stages, Summary,
    TranscriptRow, parse_answer, row_at, timestamp,
};

use crate::TranscriptLine;
use crate::{ChatBlock, ChatChip, ChatMessage, MeetingRow, SpeakerRow, StageRow, SummaryLine};

const UNTITLED: &str = "Untitled meeting";
const STOPPED_NOTE: &str = "Stopped.";
const PARTIAL_NOTE: &str = "The meeting is too long to read whole, so this looked only at the parts closest to the question.";
const UNBACKED_NOTE: &str =
    "No time in this answer matches a line of the transcript. Check it there.";

pub fn when_text(created_ms: u64, length_ms: u64) -> String {
    let when = Local
        .timestamp_millis_opt(created_ms as i64)
        .single()
        .map(|when| when.format("%-d %b %Y, %H:%M").to_string())
        .unwrap_or_default();
    format!("{when} · {}", length_text(length_ms))
}

pub fn length_text(length_ms: u64) -> String {
    let minutes = (length_ms + 30_000) / 60_000;
    match minutes {
        0 => "under a minute".to_owned(),
        1..=59 => format!("{minutes} min"),
        _ => format!("{} h {} min", minutes / 60, minutes % 60),
    }
}

pub fn meeting_row(
    entry: &MeetingEntry,
    status: &MeetingsStatus,
    hit: Option<&SearchHit>,
) -> MeetingRow {
    MeetingRow {
        id: entry.id.as_str().into(),
        title: entry.title.as_deref().unwrap_or(UNTITLED).into(),
        when: when_text(entry.created_ms, entry.length_ms).into(),
        snippet: hit
            .map(|hit| hit.snippet.as_str())
            .unwrap_or_default()
            .into(),
        hit_row: hit.and_then(|hit| hit.row).map_or(-1, |row| row as i32),
        recording: status.recording.as_deref() == Some(&entry.id),
        busy: status.work.iter().any(|(id, _)| *id == entry.id),
        failed: entry.has_failure(),
    }
}

/// What's going on, and how far along it is (0..1), or below 0 when that can't be said.
pub fn step_text(step: WorkStep) -> (&'static str, f32) {
    let fraction = |done: u64, total: u64| done as f32 / total.max(1) as f32;
    match step {
        WorkStep::Waiting => ("Waiting for the meeting before this one", -1.0),
        WorkStep::Finishing => ("Writing the end of the live transcript", -1.0),
        WorkStep::Stage(Stage::Diarization) => ("Telling the speakers apart", -1.0),
        WorkStep::Stage(Stage::Refine) => ("Going over the transcript again", -1.0),
        WorkStep::Stage(Stage::Summary) => ("Writing the summary", -1.0),
        WorkStep::Refining { done_ms, total_ms } => (
            "Going over the transcript again",
            fraction(done_ms, total_ms),
        ),
        WorkStep::Summarizing { done, total } => {
            ("Writing the summary", fraction(done as u64, total as u64))
        }
    }
}

pub fn stage_rows(stages: &Stages) -> Vec<StageRow> {
    let row = |label: &str, stage: &StageState| {
        let (state, reason) = match stage {
            StageState::Pending => (0, ""),
            StageState::Done { .. } => (1, ""),
            StageState::Skipped { reason } => (2, reason.as_str()),
            StageState::Failed { reason } => (3, reason.as_str()),
        };
        StageRow {
            label: label.into(),
            state,
            reason: reason.into(),
        }
    };
    vec![
        row("Speakers", &stages.diarization),
        row("Second pass", &stages.refine),
        row("Summary", &stages.summary),
    ]
}

/// What failed and why, a line for each.
pub fn failures_text(stages: &Stages) -> String {
    let rows = stage_rows(stages);
    let failed = rows.iter().filter(|row| row.state == 3);
    let lines: Vec<String> = failed
        .map(|row| format!("{}: {}", row.label, row.reason))
        .collect();
    lines.join("\n")
}

fn heading(text: &str) -> SummaryLine {
    SummaryLine {
        kind: 0,
        text: text.into(),
        time: "".into(),
        row: -1,
    }
}

fn item(text: String, at_ms: Option<u64>, rows: &[TranscriptRow]) -> SummaryLine {
    SummaryLine {
        kind: 1,
        text: text.into(),
        time: at_ms.map(timestamp).unwrap_or_default().into(),
        row: at_ms
            .and_then(|at_ms| row_at(rows, at_ms))
            .map_or(-1, |row| row as i32),
    }
}

pub fn summary_lines(summary: &Summary, rows: &[TranscriptRow]) -> Vec<SummaryLine> {
    let mut lines = vec![SummaryLine {
        kind: 2,
        text: summary.tldr.as_str().into(),
        time: "".into(),
        row: -1,
    }];
    if !summary.key_points.is_empty() {
        lines.push(heading("Key points"));
        let points = summary.key_points.iter();
        lines.extend(points.map(|point| item(point.text.clone(), point.at_ms, rows)));
    }
    if !summary.decisions.is_empty() {
        lines.push(heading("Decisions"));
        let decisions = summary.decisions.iter();
        lines.extend(decisions.map(|decision| item(decision.text.clone(), decision.at_ms, rows)));
    }
    if !summary.action_items.is_empty() {
        lines.push(heading("Action items"));
        lines.extend(summary.action_items.iter().map(|action| {
            let owner = action.owner.as_deref().map(|owner| format!("{owner}: "));
            let due = action.due.as_deref().map(|due| format!(" ({due})"));
            let text = format!(
                "{}{}{}",
                owner.unwrap_or_default(),
                action.task,
                due.unwrap_or_default()
            );
            item(text, action.at_ms, rows)
        }));
    }
    lines
}

pub fn transcript_line(row: &TranscriptRow) -> TranscriptLine {
    TranscriptLine {
        time: timestamp(row.at_ms).into(),
        speaker: row.name.as_str().into(),
        text: row.text.as_str().into(),
        gap: row.gap,
    }
}

pub fn speaker_rows(detail: &MeetingDetail) -> Vec<SpeakerRow> {
    let speakers = detail.speakers.iter().map(|(id, name)| SpeakerRow {
        id: id.0 as i32,
        label: format!("Speaker {}", id.0).into(),
        name: name.as_deref().unwrap_or_default().into(),
    });
    speakers.collect()
}

/// One question and its answer, with every citation checked against `rows` as they are now.
pub fn chat_message(turn: &ChatTurn, rows: &[TranscriptRow]) -> ChatMessage {
    let parsed = parse_answer(&turn.answer, rows);
    let note = if turn.stopped {
        STOPPED_NOTE
    } else if parsed.is_unbacked() {
        UNBACKED_NOTE
    } else if turn.partial_transcript && !parsed.not_found {
        PARTIAL_NOTE
    } else {
        ""
    };
    let blocks: Vec<ChatBlock> = parsed
        .blocks
        .iter()
        .map(|block| {
            let chips: Vec<ChatChip> = block
                .citations
                .iter()
                .map(|citation| ChatChip {
                    label: citation.label.as_str().into(),
                    row: citation.row.map_or(-1, |row| row as i32),
                })
                .collect();
            ChatBlock {
                text: block.text.as_str().into(),
                chips: ModelRc::new(VecModel::from(chips)),
            }
        })
        .collect();
    ChatMessage {
        question: turn.question.as_str().into(),
        blocks: ModelRc::new(VecModel::from(blocks)),
        note: note.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(at_ms: u64, text: &str) -> TranscriptRow {
        TranscriptRow {
            at_ms,
            speaker: None,
            name: "You".to_owned(),
            text: text.to_owned(),
            gap: false,
        }
    }

    fn turn(answer: &str) -> ChatTurn {
        ChatTurn {
            question: "What now?".to_owned(),
            answer: answer.to_owned(),
            asked_ms: 0,
            stopped: false,
            partial_transcript: false,
        }
    }

    #[test]
    fn lengths_read_like_a_person_would_say_them() {
        assert_eq!(length_text(20_000), "under a minute");
        assert_eq!(length_text(32 * 60_000), "32 min");
        assert_eq!(length_text(65 * 60_000), "1 h 5 min");
    }

    #[test]
    fn a_real_citation_links_and_a_made_up_one_does_not() {
        let rows = [line(5_000, "We ship on Friday.")];

        let message = chat_message(&turn("Friday. [00:05] [09:09]"), &rows);

        use slint::Model;
        let block = message.blocks.row_data(0).unwrap();
        assert_eq!(block.text, "Friday.");
        assert_eq!(block.chips.row_data(0).unwrap().row, 0);
        assert_eq!(block.chips.row_data(1).unwrap().row, -1);
        assert_eq!(message.note, "");
    }

    #[test]
    fn an_answer_with_nothing_behind_it_says_so() {
        let rows = [line(5_000, "We ship on Friday.")];
        let message = chat_message(&turn("Monday. [09:09]"), &rows);
        assert_eq!(message.note, UNBACKED_NOTE);
    }

    #[test]
    fn not_in_the_meeting_is_an_answer_and_needs_no_warning() {
        let rows = [line(5_000, "We ship on Friday.")];
        let message = chat_message(&turn("That isn't in this meeting."), &rows);
        assert_eq!(message.note, "");
    }

    #[test]
    fn a_stopped_answer_says_it_was_stopped() {
        let stopped = ChatTurn {
            stopped: true,
            ..turn("Fri")
        };
        assert_eq!(chat_message(&stopped, &[]).note, STOPPED_NOTE);
    }

    #[test]
    fn progress_is_a_fraction_only_when_there_is_one() {
        let refining = WorkStep::Refining {
            done_ms: 500,
            total_ms: 1_000,
        };
        assert_eq!(step_text(refining).1, 0.5);
        assert!(step_text(WorkStep::Waiting).1 < 0.0);
    }
}
