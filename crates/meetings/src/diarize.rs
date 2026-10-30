//! Speakers on the Others channel: turning what the engine found into ids that hold for the whole
//! meeting, and placing them on the transcript.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use wf_pipeline::SpeakerTurn;

use crate::Channel;
use crate::transcript::{Segment, SpeakerId};

/// Shorter than this, a piece of a chunk is too little for ASR and likely a misheard change.
const MIN_PIECE_MS: u64 = 1_000;

/// One stretch of one voice, on the meeting's timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker: SpeakerId,
}

impl Turn {
    fn overlap(&self, start_ms: u64, end_ms: u64) -> u64 {
        self.end_ms
            .min(end_ms)
            .saturating_sub(self.start_ms.max(start_ms))
    }
}

/// Puts the engine's turns on ids that last. Its numbers mean nothing from one run to the next,
/// so each cluster takes the id of the old voice it overlaps most, biggest overlap first, and
/// anything left gets a fresh id. Old ids are never reused: a name stays with its voice.
pub fn stable_turns(previous: &[Turn], found: &[SpeakerTurn]) -> Vec<Turn> {
    let mut clusters: Vec<u32> = Vec::new();
    for turn in found {
        if !clusters.contains(&turn.speaker) {
            clusters.push(turn.speaker);
        }
    }

    let mut overlaps: BTreeMap<(u32, SpeakerId), u64> = BTreeMap::new();
    for turn in found {
        let (start_ms, end_ms) = (turn.start.as_millis() as u64, turn.end.as_millis() as u64);
        for old in previous {
            let overlap = old.overlap(start_ms, end_ms);
            if overlap > 0 {
                *overlaps.entry((turn.speaker, old.speaker)).or_default() += overlap;
            }
        }
    }
    let mut candidates: Vec<((u32, SpeakerId), u64)> = overlaps.into_iter().collect();
    candidates.sort_by_key(|&(key, overlap)| (std::cmp::Reverse(overlap), key));

    let mut ids: BTreeMap<u32, SpeakerId> = BTreeMap::new();
    let mut taken = BTreeSet::new();
    for ((cluster, old), _) in candidates {
        if !ids.contains_key(&cluster) && !taken.contains(&old) {
            ids.insert(cluster, old);
            taken.insert(old);
        }
    }

    let mut next = previous.iter().map(|t| t.speaker.0).max().unwrap_or(0) + 1;
    for cluster in clusters {
        ids.entry(cluster).or_insert_with(|| {
            let id = SpeakerId(next);
            next += 1;
            id
        });
    }

    found
        .iter()
        .map(|turn| Turn {
            start_ms: turn.start.as_millis() as u64,
            end_ms: turn.end.as_millis() as u64,
            speaker: ids[&turn.speaker],
        })
        .collect()
}

/// Cuts a chunk of audio where the speaker changes, so each piece has one voice for ASR. Every
/// millisecond of the chunk lands in one piece. `turns` are in order of start.
pub fn split_at_turns(
    start_ms: u64,
    end_ms: u64,
    turns: &[Turn],
) -> Vec<(u64, u64, Option<SpeakerId>)> {
    let mut pieces: Vec<Turn> = Vec::new();
    for turn in turns {
        if turn.overlap(start_ms, end_ms) == 0 {
            continue;
        }
        let mut piece = Turn {
            start_ms: turn.start_ms.max(start_ms),
            end_ms: turn.end_ms.min(end_ms),
            speaker: turn.speaker,
        };
        if let Some(last) = pieces.last_mut() {
            if piece.end_ms <= last.end_ms {
                continue;
            }
            piece.start_ms = piece.start_ms.max(last.end_ms);
            if piece.speaker == last.speaker {
                last.end_ms = piece.end_ms;
                continue;
            }
        }
        pieces.push(piece);
    }
    if pieces.is_empty() {
        return vec![(start_ms, end_ms, None)];
    }

    // Slivers go to the neighbour before them, or after them at the very start.
    let mut kept: Vec<Turn> = Vec::new();
    for piece in pieces {
        let sliver = piece.end_ms - piece.start_ms < MIN_PIECE_MS;
        match kept.last_mut() {
            Some(last) if sliver || last.speaker == piece.speaker => last.end_ms = piece.end_ms,
            _ => kept.push(piece),
        }
    }
    if kept.len() > 1 && kept[0].end_ms - kept[0].start_ms < MIN_PIECE_MS {
        let first = kept.remove(0);
        kept[0].start_ms = first.start_ms;
    }

    // Close the pauses, so no audio falls between two pieces.
    let mut split = Vec::with_capacity(kept.len());
    let mut at = start_ms;
    for (n, piece) in kept.iter().enumerate() {
        let end = kept
            .get(n + 1)
            .map_or(end_ms, |next| piece.end_ms.midpoint(next.start_ms));
        split.push((at, end, Some(piece.speaker)));
        at = end;
    }
    split
}

/// Gives each Others segment the speaker heard most during it. For transcripts that weren't cut
/// at speaker changes.
pub fn assign_speakers(segments: &mut [Segment], turns: &[Turn]) {
    for segment in segments {
        if segment.channel != Channel::Others {
            continue;
        }
        let mut totals: BTreeMap<SpeakerId, u64> = BTreeMap::new();
        for turn in turns {
            let overlap = turn.overlap(segment.start_ms, segment.end_ms);
            if overlap > 0 {
                *totals.entry(turn.speaker).or_default() += overlap;
            }
        }
        segment.speaker = totals
            .into_iter()
            .max_by_key(|&(_, total)| total)
            .map(|(speaker, _)| speaker);
    }
}
