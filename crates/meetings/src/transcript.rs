//! What a meeting's transcript is made of. Times are milliseconds on the meeting's timeline,
//! which keeps running across a resumed recording.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Channel;

/// One voice on the Others channel, stable within a meeting. Shown as "Speaker N" until renamed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SpeakerId(pub u32);

/// Renames, kept apart from the transcript so regenerating it never loses them.
pub type SpeakerNames = BTreeMap<SpeakerId, String>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub channel: Channel,
    /// Set once diarization has placed this segment. Never set on the You channel.
    pub speaker: Option<SpeakerId>,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

impl Segment {
    /// The stable label: "You", "Others" or "Speaker N".
    pub fn label(&self) -> String {
        label(self.channel, self.speaker)
    }

    /// The label with any rename applied.
    pub fn display_name(&self, names: &SpeakerNames) -> String {
        self.speaker
            .and_then(|id| names.get(&id).cloned())
            .unwrap_or_else(|| self.label())
    }
}

/// A stretch of a channel with no text yet, so the transcript shows the hole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gap {
    pub channel: Channel,
    pub start_ms: u64,
    pub end_ms: u64,
    pub reason: GapReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GapReason {
    /// ASR fell too far behind and this audio was skipped.
    Behind,
    /// ASR failed on this audio.
    Failed,
    /// The app stopped before this audio was transcribed.
    Interrupted,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcript {
    /// In order of `start_ms`.
    pub segments: Vec<Segment>,
    pub gaps: Vec<Gap>,
}

impl Transcript {
    /// One `[mm:ss] Name: text` line per segment, with gaps marked where they fall.
    pub fn lines(&self, names: &SpeakerNames) -> Vec<String> {
        let segments = self.segments.iter().map(|segment| {
            let line = format!(
                "{} {}: {}",
                timestamp(segment.start_ms),
                segment.display_name(names),
                segment.text
            );
            (segment.start_ms, line)
        });
        let gaps = self.gaps.iter().map(|gap| {
            let seconds = gap.end_ms.saturating_sub(gap.start_ms).div_ceil(1_000);
            let line = format!(
                "{} {}: (not transcribed, {seconds} s)",
                timestamp(gap.start_ms),
                label(gap.channel, None),
            );
            (gap.start_ms, line)
        });
        let mut lines: Vec<(u64, String)> = segments.chain(gaps).collect();
        lines.sort_by_key(|&(start, _)| start);
        lines.into_iter().map(|(_, line)| line).collect()
    }

    pub(crate) fn sort(&mut self) {
        self.segments.sort_by_key(|s| (s.start_ms, s.channel));
        self.gaps.sort_by_key(|g| (g.start_ms, g.channel));
    }
}

/// `[mm:ss]`. Minutes keep counting past the hour, so citations stay one shape.
pub fn timestamp(ms: u64) -> String {
    let seconds = ms / 1_000;
    format!("[{:02}:{:02}]", seconds / 60, seconds % 60)
}

/// Reads a [`timestamp`] back, e.g. out of a chat answer's citation.
pub fn parse_timestamp(text: &str) -> Option<u64> {
    let (minutes, seconds) = text.strip_prefix('[')?.strip_suffix(']')?.split_once(':')?;
    if minutes.len() < 2 || seconds.len() != 2 {
        return None;
    }
    let minutes: u64 = minutes.parse().ok()?;
    let seconds: u64 = seconds.parse().ok()?;
    (seconds < 60).then_some((minutes * 60 + seconds) * 1_000)
}

fn label(channel: Channel, speaker: Option<SpeakerId>) -> String {
    match (channel, speaker) {
        (Channel::You, _) => "You".to_owned(),
        (Channel::Others, None) => "Others".to_owned(),
        (Channel::Others, Some(SpeakerId(number))) => format!("Speaker {number}"),
    }
}
