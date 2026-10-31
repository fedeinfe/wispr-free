//! What the Meetings window shows, as Swift hears it. Unlike the Settings pages this can't go out
//! whole on every change, as a meeting's transcript would cross again with every token of an
//! answer, so each thing that changed goes out by itself.

use crate::lists::Rows;

use super::Shown;
use super::rows::{
    ChatMessage, MeetingRow, SpeakerRow, StageRow, SummaryLine, TranscriptLine, streaming_line,
};

/// Where in a list the rows that came with this go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RowsPlace {
    /// They're the whole list.
    All,
    /// The one row replaces this one.
    At { index: u32 },
    /// The one row goes on the end.
    End,
    /// The one row goes in before this one.
    Before { index: u32 },
}

fn placed<T>(change: Rows<T>) -> (RowsPlace, Vec<T>) {
    match change {
        Rows::All(rows) => (RowsPlace::All, rows),
        Rows::One(index, row) => (
            RowsPlace::At {
                index: index as u32,
            },
            vec![row],
        ),
        Rows::Add(row) => (RowsPlace::End, vec![row]),
        Rows::Insert(index, row) => (
            RowsPlace::Before {
                index: index as u32,
            },
            vec![row],
        ),
    }
}

/// One thing for the window to show now, as [`Shown`] has it.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum MeetingsChange {
    Meetings {
        place: RowsPlace,
        rows: Vec<MeetingRow>,
    },
    ListHint {
        text: String,
    },
    DetailHint {
        text: String,
    },
    Recording {
        recording: bool,
    },
    StartLabel {
        text: String,
    },
    SelectedId {
        id: String,
    },
    Title {
        text: String,
    },
    Notes {
        text: String,
    },
    When {
        text: String,
    },
    ThisRecording {
        recording: bool,
    },
    Busy {
        busy: bool,
    },
    CanFinish {
        can: bool,
    },
    CanResume {
        can: bool,
    },
    CanSummarize {
        can: bool,
    },
    CanExport {
        can: bool,
    },
    CanDelete {
        can: bool,
    },
    Stages {
        rows: Vec<StageRow>,
    },
    Failures {
        text: String,
    },
    ProgressText {
        text: String,
    },
    /// 0..1, or below 0 when that can't be said.
    Progress {
        fraction: f32,
    },
    Notice {
        text: String,
    },
    /// Summary, Notes, Transcript, Chat, counted from 0.
    Tab {
        index: i32,
    },
    /// Shows a line of the transcript.
    Jump {
        row: i32,
    },
    Summary {
        lines: Vec<SummaryLine>,
    },
    SummaryEmpty {
        text: String,
    },
    SummaryPartial {
        partial: bool,
    },
    SummaryStale {
        stale: bool,
    },
    Transcript {
        place: RowsPlace,
        rows: Vec<TranscriptLine>,
    },
    TranscriptEmpty {
        text: String,
    },
    Speakers {
        rows: Vec<SpeakerRow>,
    },
    Chat {
        place: RowsPlace,
        rows: Vec<ChatMessage>,
    },
    Suggestions {
        questions: Vec<String>,
    },
    ChatHint {
        text: String,
    },
    ChatRunning {
        running: bool,
    },
    StreamingQuestion {
        text: String,
    },
    StreamingText {
        text: String,
    },
    ChatError {
        text: String,
    },
}

impl From<Shown> for MeetingsChange {
    fn from(shown: Shown) -> Self {
        use MeetingsChange as Change;
        match shown {
            Shown::Meetings(change) => {
                let (place, rows) = placed(change);
                Change::Meetings { place, rows }
            }
            Shown::ListHint(text) => Change::ListHint { text },
            Shown::DetailHint(text) => Change::DetailHint { text },
            Shown::Recording(recording) => Change::Recording { recording },
            Shown::StartLabel(text) => Change::StartLabel { text },
            Shown::SelectedId(id) => Change::SelectedId { id },
            Shown::Title(text) => Change::Title { text },
            Shown::Notes(text) => Change::Notes { text },
            Shown::When(text) => Change::When { text },
            Shown::ThisRecording(recording) => Change::ThisRecording { recording },
            Shown::Busy(busy) => Change::Busy { busy },
            Shown::CanFinish(can) => Change::CanFinish { can },
            Shown::CanResume(can) => Change::CanResume { can },
            Shown::CanSummarize(can) => Change::CanSummarize { can },
            Shown::CanExport(can) => Change::CanExport { can },
            Shown::CanDelete(can) => Change::CanDelete { can },
            Shown::Stages(rows) => Change::Stages { rows },
            Shown::Failures(text) => Change::Failures { text },
            Shown::ProgressText(text) => Change::ProgressText { text },
            Shown::Progress(fraction) => Change::Progress { fraction },
            Shown::Notice(text) => Change::Notice { text },
            Shown::Tab(index) => Change::Tab { index },
            Shown::Jump(row) => Change::Jump { row },
            Shown::Summary(lines) => Change::Summary { lines },
            Shown::SummaryEmpty(text) => Change::SummaryEmpty { text },
            Shown::SummaryPartial(partial) => Change::SummaryPartial { partial },
            Shown::SummaryStale(stale) => Change::SummaryStale { stale },
            Shown::Transcript(change) => {
                let (place, rows) = placed(change);
                Change::Transcript { place, rows }
            }
            Shown::TranscriptEmpty(text) => Change::TranscriptEmpty { text },
            Shown::Speakers(rows) => Change::Speakers { rows },
            Shown::Chat(change) => {
                let (place, rows) = placed(change);
                Change::Chat { place, rows }
            }
            Shown::Suggestions(questions) => Change::Suggestions { questions },
            Shown::ChatHint(text) => Change::ChatHint { text },
            Shown::ChatRunning(running) => Change::ChatRunning { running },
            Shown::StreamingQuestion(text) => Change::StreamingQuestion { text },
            Shown::StreamingText(text) => Change::StreamingText {
                text: streaming_line(&text),
            },
            Shown::ChatError(text) => Change::ChatError { text },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> TranscriptLine {
        TranscriptLine {
            text: text.to_owned(),
            ..TranscriptLine::default()
        }
    }

    #[test]
    fn each_kind_of_list_change_says_where_it_goes() {
        let place = |change| placed(change).0;

        assert_eq!(place(Rows::All(vec![line("a")])), RowsPlace::All);
        assert_eq!(place(Rows::Add(line("a"))), RowsPlace::End);
        assert_eq!(
            place(Rows::One(2, line("a"))),
            RowsPlace::At { index: 2 }
        );
        assert_eq!(
            place(Rows::Insert(3, line("a"))),
            RowsPlace::Before { index: 3 }
        );
    }

    #[test]
    fn a_line_inserted_keeps_its_text() {
        let change = MeetingsChange::from(Shown::Transcript(Rows::Insert(0, line("Hello"))));
        assert_eq!(
            change,
            MeetingsChange::Transcript {
                place: RowsPlace::Before { index: 0 },
                rows: vec![line("Hello")],
            }
        );
    }

    #[test]
    fn an_answer_with_no_words_yet_says_it_is_thinking() {
        let change = MeetingsChange::from(Shown::StreamingText(String::new()));
        assert_eq!(
            change,
            MeetingsChange::StreamingText {
                text: "Thinking…".to_owned()
            }
        );
    }
}
