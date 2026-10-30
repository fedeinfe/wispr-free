//! The meetings as the Meetings window sees them: a list, one meeting in full, search and export.
//! Nothing here knows about the window. Everything reads the disk, so call it from a worker thread.

use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use crate::Error;
use crate::chat::ChatTurn;
use crate::store::{MeetingRecord, MeetingStore, StageState, Stages};
use crate::summary::{Summary, rename_speaker_labels};
use crate::transcript::{SpeakerId, TranscriptRow, timestamp};

/// The characters of context a search result shows around what it found.
const SNIPPET_CHARS: usize = 80;
/// A file name longer than this is hard to read in a folder.
const MAX_FILE_NAME_CHARS: usize = 60;
const NO_TITLE: &str = "Meeting";

/// One line of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingEntry {
    pub id: String,
    /// The user's title, then the summary's.
    pub title: Option<String>,
    pub created_ms: u64,
    pub length_ms: u64,
    pub stages: Stages,
}

impl MeetingEntry {
    /// A stage said why it couldn't run.
    pub fn has_failure(&self) -> bool {
        stage_states(&self.stages)
            .iter()
            .any(|state| matches!(state, StageState::Failed { .. }))
    }
}

fn stage_states(stages: &Stages) -> [&StageState; 3] {
    [&stages.diarization, &stages.refine, &stages.summary]
}

fn title_of(record: &MeetingRecord, summary: Option<&Summary>) -> Option<String> {
    record
        .title
        .clone()
        .or_else(|| summary.and_then(|summary| summary.title.clone()))
}

fn length_ms(record: &MeetingRecord) -> u64 {
    record
        .parts
        .iter()
        .filter_map(|part| part.duration_ms)
        .sum::<u64>()
}

/// Every meeting, newest first.
pub fn meeting_entries(store: &MeetingStore) -> Result<Vec<MeetingEntry>, Error> {
    store
        .list()?
        .into_iter()
        .map(|record| {
            let summary = store.open(&record.id)?.summary()?;
            Ok(MeetingEntry {
                title: title_of(&record, summary.as_ref()),
                length_ms: length_ms(&record),
                created_ms: record.created_ms,
                stages: record.stages,
                id: record.id,
            })
        })
        .collect()
}

/// One meeting, whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingDetail {
    pub record: MeetingRecord,
    pub title: Option<String>,
    pub length_ms: u64,
    /// With the current speaker names.
    pub summary: Option<Summary>,
    /// The transcript has moved on since: offer "Update summary".
    pub summary_stale: bool,
    pub notes: String,
    pub rows: Vec<TranscriptRow>,
    /// Every voice in the transcript, by number, with its name if it has one.
    pub speakers: Vec<(SpeakerId, Option<String>)>,
    pub chat: Vec<ChatTurn>,
    /// A recording was cut off, or the stages after it didn't finish.
    pub unfinished: bool,
}

/// Reads one meeting.
pub fn meeting_detail(store: &MeetingStore, id: &str) -> Result<MeetingDetail, Error> {
    let meeting = store.open(id)?;
    let record = meeting.record().clone();
    let summary = meeting.summary()?;
    let transcript = meeting.transcript()?;
    let rows = transcript.rows(&record.speakers);
    let mut voices: Vec<SpeakerId> = rows.iter().filter_map(|row| row.speaker).collect();
    voices.sort_unstable();
    voices.dedup();
    let speakers = voices
        .into_iter()
        .map(|id| (id, record.speakers.get(&id).cloned()))
        .collect();
    let unfinished = !meeting.is_recording()
        && !record.parts.is_empty()
        && stage_states(&record.stages)
            .iter()
            .any(|state| matches!(state, StageState::Pending));
    Ok(MeetingDetail {
        title: title_of(&record, summary.as_ref()),
        length_ms: length_ms(&record),
        summary_stale: meeting.summary_is_stale()?,
        notes: meeting.notes()?,
        chat: meeting.chat()?,
        rows,
        speakers,
        summary,
        unfinished,
        record,
    })
}

/// Where a search found something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub id: String,
    /// What to show under the title.
    pub snippet: String,
    /// The transcript row, when that's where it was.
    pub row: Option<usize>,
}

struct Indexed {
    id: String,
    title: String,
    /// Summary then notes, kept apart from the transcript so they're shown first.
    written: Vec<String>,
    rows: Vec<TranscriptRow>,
    /// Everything, folded, for finding the words.
    folded: String,
}

/// Every meeting's words, read once, so that each key press doesn't go back to the disk.
#[derive(Default)]
pub struct SearchIndex(Vec<Indexed>);

impl SearchIndex {
    pub fn build(store: &MeetingStore) -> Result<Self, Error> {
        let mut meetings = Vec::new();
        for record in store.list()? {
            let detail = meeting_detail(store, &record.id)?;
            let mut written = Vec::new();
            if let Some(summary) = &detail.summary {
                written.push(summary_text(summary));
            }
            if !detail.notes.trim().is_empty() {
                written.push(detail.notes.clone());
            }
            let title = detail.title.clone().unwrap_or_default();
            let mut all = vec![title.clone()];
            all.extend(written.iter().cloned());
            all.extend(
                detail
                    .rows
                    .iter()
                    .map(|row| format!("{} {}", row.name, row.text)),
            );
            meetings.push(Indexed {
                id: record.id,
                title,
                written,
                rows: detail.rows,
                folded: fold(&all.join("\n")),
            });
        }
        Ok(Self(meetings))
    }

    /// The meetings holding every word of `query`, newest first. Case and accents don't count.
    pub fn search(&self, query: &str) -> Vec<SearchHit> {
        let query = fold(query);
        let words: Vec<&str> = query.split_whitespace().collect();
        let Some(first) = words.first() else {
            return Vec::new();
        };
        self.0
            .iter()
            .filter(|meeting| words.iter().all(|word| meeting.folded.contains(word)))
            .map(|meeting| meeting.hit(first))
            .collect()
    }
}

impl Indexed {
    fn hit(&self, word: &str) -> SearchHit {
        let in_title = fold(&self.title).contains(word);
        let written = self.written.iter().find(|text| fold(text).contains(word));
        if in_title || written.is_some() {
            let shown = match written {
                Some(text) if !in_title => snippet(text, word),
                _ => String::new(),
            };
            return SearchHit {
                id: self.id.clone(),
                snippet: shown,
                row: None,
            };
        }
        let found = self
            .rows
            .iter()
            .position(|row| fold(&format!("{} {}", row.name, row.text)).contains(word));
        SearchHit {
            id: self.id.clone(),
            snippet: found.map_or_else(String::new, |row| {
                let row = &self.rows[row];
                format!("{} {}", timestamp(row.at_ms), snippet(&row.text, word))
            }),
            row: found,
        }
    }
}

/// Lowercase, without the accents Latin languages put on letters.
fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            other => other,
        })
        .collect()
}

/// The stretch of `text` around `word`. Folding keeps one character for one, so places carry over.
fn snippet(text: &str, word: &str) -> String {
    let line = text
        .lines()
        .find(|line| fold(line).contains(word))
        .unwrap_or(text);
    let chars: Vec<char> = line.chars().collect();
    let folded: Vec<char> = chars
        .iter()
        .map(|c| fold(&c.to_string()).chars().next().unwrap_or(*c))
        .collect();
    let wanted: Vec<char> = word.chars().collect();
    let at = folded
        .windows(wanted.len().max(1))
        .position(|window| window == wanted)
        .unwrap_or(0);
    let start = at.saturating_sub(SNIPPET_CHARS / 2);
    let end = (at + SNIPPET_CHARS).min(chars.len());
    let mut piece: String = chars[start..end].iter().collect();
    if start > 0 {
        piece.insert(0, '…');
    }
    if end < chars.len() {
        piece.push('…');
    }
    piece
}

fn summary_text(summary: &Summary) -> String {
    let mut lines = vec![summary.tldr.clone()];
    let items = summary.key_points.iter().chain(&summary.decisions);
    lines.extend(items.map(|item| item.text.clone()));
    lines.extend(summary.action_items.iter().map(|item| match &item.owner {
        Some(owner) => format!("{owner}: {}", item.task),
        None => item.task.clone(),
    }));
    lines.join("\n")
}

/// The meeting as a Markdown file. `when` is its date, as the reader's locale writes it.
pub fn export_markdown(detail: &MeetingDetail, when: &str) -> String {
    let names = &detail.record.speakers;
    let mut text = format!(
        "# {}\n\n{when} · {}\n",
        detail.title.as_deref().unwrap_or(NO_TITLE),
        timestamp(detail.length_ms).trim_matches(['[', ']']),
    );
    if let Some(summary) = &detail.summary {
        text.push_str("\n## Summary\n\n");
        text.push_str(
            "*Written by a local model, and it can be wrong. The transcript is the record.*\n\n",
        );
        text.push_str(&summary.tldr);
        text.push('\n');
        let time = |at: Option<u64>| {
            at.map(|ms| format!("{} ", timestamp(ms)))
                .unwrap_or_default()
        };
        let list = |heading: &str, lines: Vec<String>| {
            if lines.is_empty() {
                return String::new();
            }
            format!("\n### {heading}\n\n{}\n", lines.join("\n"))
        };
        text.push_str(&list(
            "Key points",
            summary
                .key_points
                .iter()
                .map(|item| format!("- {}{}", time(item.at_ms), item.text))
                .collect(),
        ));
        text.push_str(&list(
            "Decisions",
            summary
                .decisions
                .iter()
                .map(|item| format!("- {}{}", time(item.at_ms), item.text))
                .collect(),
        ));
        text.push_str(&list(
            "Action items",
            summary
                .action_items
                .iter()
                .map(|item| {
                    let owner = item
                        .owner
                        .as_deref()
                        .map(|o| format!("{o}: "))
                        .unwrap_or_default();
                    let due = item
                        .due
                        .as_deref()
                        .map(|d| format!(" ({d})"))
                        .unwrap_or_default();
                    format!("- {}{owner}{}{due}", time(item.at_ms), item.task)
                })
                .collect(),
        ));
    }
    if !detail.notes.trim().is_empty() {
        text.push_str("\n## Notes\n\n");
        text.push_str(detail.notes.trim());
        text.push('\n');
    }
    if !detail.rows.is_empty() {
        text.push_str("\n## Transcript\n\n");
        for row in &detail.rows {
            let words = rename_speaker_labels(&row.text, names);
            text.push_str(&format!(
                "{} {}: {words}\n\n",
                timestamp(row.at_ms),
                row.name
            ));
        }
    }
    text
}

/// A name for the export that every file system takes.
pub fn export_file_name(title: Option<&str>) -> String {
    let title = title.unwrap_or(NO_TITLE);
    let safe: String = title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .take(MAX_FILE_NAME_CHARS)
        .collect();
    let safe = safe.split_whitespace().collect::<Vec<_>>().join(" ");
    let safe = safe.trim_matches('.');
    if safe.is_empty() {
        NO_TITLE.to_owned()
    } else {
        safe.to_owned()
    }
}

/// Writes the export into `folder` under a new name, never over another file, and returns where.
pub fn write_export(folder: &Path, detail: &MeetingDetail, when: &str) -> Result<PathBuf, Error> {
    std::fs::create_dir_all(folder)?;
    let name = export_file_name(detail.title.as_deref());
    let text = export_markdown(detail, when);
    for attempt in 1.. {
        let file = match attempt {
            1 => format!("{name}.md"),
            n => format!("{name} {n}.md"),
        };
        let path = folder.join(file);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut output) => {
                output.write_all(text.as_bytes())?;
                return Ok(path);
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
    }
    unreachable!("the loop only ends by returning")
}
