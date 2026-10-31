//! What the window shows, worked out from the meeting data. No window here, so all of it can be
//! tested as text.

use chrono::{Local, TimeZone};
use wf_dictation::{MeetingsStatus, WorkStep};
use wf_meetings::{
    ChatTurn, MeetingDetail, MeetingEntry, SearchHit, Stage, StageState, Stages, Summary,
    TranscriptRow, parse_answer, row_at, timestamp,
};

const UNTITLED: &str = "Untitled meeting";
const STOPPED_NOTE: &str = "Stopped.";
const PARTIAL_NOTE: &str = "The meeting is too long to read whole, so this looked only at the parts closest to the question.";
const UNBACKED_NOTE: &str =
    "No time in this answer matches a line of the transcript. Check it there.";
const THINKING: &str = "Thinking…";

#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct MeetingRow {
    pub id: String,
    pub title: String,
    pub when: String,
    /// Why a search found it. Empty outside a search.
    pub snippet: String,
    /// Where in the transcript the search found it, or -1.
    pub hit_row: i32,
    pub recording: bool,
    pub busy: bool,
    pub failed: bool,
}

/// How a stage after the meeting stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, uniffi::Enum)]
pub enum StageMark {
    #[default]
    Waiting,
    Done,
    Skipped,
    Failed,
}

impl From<StageMark> for i32 {
    fn from(mark: StageMark) -> Self {
        mark as i32
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct StageRow {
    pub label: String,
    pub state: StageMark,
    pub reason: String,
    /// The stage as the status row says it.
    pub text: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, uniffi::Enum)]
pub enum LineKind {
    Heading,
    Item,
    #[default]
    Plain,
}

impl From<LineKind> for i32 {
    fn from(kind: LineKind) -> Self {
        kind as i32
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct SummaryLine {
    pub kind: LineKind,
    pub text: String,
    /// Empty when the line has no time.
    pub time: String,
    /// The transcript line it points to, or -1 if there is none.
    pub row: i32,
    /// The time as a chip, when the line has one.
    pub chip: Option<ChatChip>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct TranscriptLine {
    pub time: String,
    pub speaker: String,
    pub text: String,
    pub gap: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct SpeakerRow {
    pub id: i32,
    pub label: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct ChatChip {
    pub label: String,
    /// The transcript line, or -1 for a time that matches none.
    pub row: i32,
    /// What it reads, which says so when no line is behind it.
    pub text: String,
    /// What it does, for an assistive tool.
    pub help: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct ChatBlock {
    pub text: String,
    pub chips: Vec<ChatChip>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct ChatMessage {
    pub question: String,
    pub blocks: Vec<ChatBlock>,
    pub note: String,
}

/// A time in the meeting. A time with no line behind it is shown as such and leads nowhere.
pub fn time_chip(label: &str, row: i32) -> ChatChip {
    let (text, help) = if row >= 0 {
        (label.to_owned(), format!("Go to {label}"))
    } else {
        (
            format!("{label} ?"),
            format!("{label} matches no line of the transcript"),
        )
    };
    ChatChip {
        label: label.to_owned(),
        row,
        text,
        help,
    }
}

/// What shows while an answer is on its way.
pub fn streaming_line(text: &str) -> String {
    if text.is_empty() { THINKING } else { text }.to_owned()
}

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
        when: when_text(entry.created_ms, entry.length_ms),
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
            StageState::Pending => (StageMark::Waiting, ""),
            StageState::Done { .. } => (StageMark::Done, ""),
            StageState::Skipped { reason } => (StageMark::Skipped, reason.as_str()),
            StageState::Failed { reason } => (StageMark::Failed, reason.as_str()),
        };
        let off = if state == StageMark::Skipped {
            " off"
        } else {
            ""
        };
        StageRow {
            label: label.into(),
            state,
            reason: reason.into(),
            text: format!("{label}{off}"),
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
    let failed = rows.iter().filter(|row| row.state == StageMark::Failed);
    let lines: Vec<String> = failed
        .map(|row| format!("{}: {}", row.label, row.reason))
        .collect();
    lines.join("\n")
}

fn heading(text: &str) -> SummaryLine {
    SummaryLine {
        kind: LineKind::Heading,
        text: text.into(),
        ..SummaryLine::default()
    }
    .pointing_nowhere()
}

fn item(text: String, at_ms: Option<u64>, rows: &[TranscriptRow]) -> SummaryLine {
    let time = at_ms.map(timestamp).unwrap_or_default();
    let row = at_ms
        .and_then(|at_ms| row_at(rows, at_ms))
        .map_or(-1, |row| row as i32);
    SummaryLine {
        kind: LineKind::Item,
        text,
        chip: at_ms.map(|_| time_chip(&time, row)),
        time,
        row,
    }
}

impl SummaryLine {
    fn pointing_nowhere(self) -> Self {
        Self { row: -1, ..self }
    }
}

pub fn summary_lines(summary: &Summary, rows: &[TranscriptRow]) -> Vec<SummaryLine> {
    let mut lines = vec![
        SummaryLine {
            kind: LineKind::Plain,
            text: summary.tldr.as_str().into(),
            ..SummaryLine::default()
        }
        .pointing_nowhere(),
    ];
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
        time: timestamp(row.at_ms),
        speaker: row.name.as_str().into(),
        text: row.text.as_str().into(),
        gap: row.gap,
    }
}

pub fn speaker_rows(detail: &MeetingDetail) -> Vec<SpeakerRow> {
    let speakers = detail.speakers.iter().map(|(id, name)| SpeakerRow {
        id: id.0 as i32,
        label: format!("Speaker {}", id.0),
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
    let blocks = parsed
        .blocks
        .iter()
        .map(|block| {
            let chips = block
                .citations
                .iter()
                .map(|citation| {
                    time_chip(&citation.label, citation.row.map_or(-1, |row| row as i32))
                })
                .collect();
            ChatBlock {
                text: block.text.as_str().into(),
                chips,
            }
        })
        .collect();
    ChatMessage {
        question: turn.question.as_str().into(),
        blocks,
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

        let block = &message.blocks[0];
        assert_eq!(block.text, "Friday.");
        assert_eq!(block.chips[0].row, 0);
        assert_eq!(block.chips[1].row, -1);
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
    fn a_time_says_whether_a_line_is_behind_it() {
        let real = time_chip("[00:12]", 1);
        assert_eq!(
            (real.text.as_str(), real.help.as_str()),
            ("[00:12]", "Go to [00:12]")
        );

        let made_up = time_chip("[09:09]", -1);
        assert_eq!(made_up.text, "[09:09] ?");
        assert_eq!(made_up.help, "[09:09] matches no line of the transcript");
    }

    #[test]
    fn a_stage_turned_off_says_so() {
        let stages = Stages {
            diarization: StageState::Skipped {
                reason: "turned off".to_owned(),
            },
            ..Stages::default()
        };
        let rows = stage_rows(&stages);
        assert_eq!(rows[0].text, "Speakers off");
        assert_eq!(rows[0].state, StageMark::Skipped);
        assert_eq!(rows[1].text, "Second pass");
    }

    #[test]
    fn an_answer_on_its_way_shows_something_from_the_start() {
        assert_eq!(streaming_line(""), "Thinking…");
        assert_eq!(streaming_line("By"), "By");
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
