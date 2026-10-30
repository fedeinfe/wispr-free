//! The scripted meeting (ticket 16) and how a summary of it is scored. Needs no model, so the
//! scoring has tests of its own.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use wf_meetings::{ActionItem, Summary};

const SCRIPT: &str = include_str!("../eval/meeting.toml");
const HOLDOUT: &str = include_str!("../eval/meeting_holdout.toml");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeetingScript {
    /// Who is on the mic. Everyone else shares the system channel.
    pub you: String,
    pub voices: BTreeMap<String, String>,
    /// Phrases for where a discussion started and didn't end. A summary presenting one of these
    /// as the outcome is wrong, however many right lines sit next to it.
    #[serde(default)]
    pub stale: Vec<String>,
    #[serde(default)]
    pub decisions: Vec<KnownDecision>,
    #[serde(default)]
    pub actions: Vec<KnownAction>,
    script: Turns,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Turns {
    turns: Vec<(String, String)>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnownDecision {
    pub id: String,
    /// A line must match every group, by any phrase in it.
    pub all: Vec<Vec<String>>,
    /// Phrases that make a line wrong.
    #[serde(default)]
    pub not: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnownAction {
    pub id: String,
    pub owner: String,
    pub task: Vec<Vec<String>>,
    /// Empty when nobody stated one.
    pub due: Vec<String>,
}

impl MeetingScript {
    pub fn bundled() -> anyhow::Result<Self> {
        toml::from_str(SCRIPT).context("eval/meeting.toml")
    }

    /// Kept out of prompt tuning. Transcript only, there's no audio for it.
    pub fn holdout() -> anyhow::Result<Self> {
        toml::from_str(HOLDOUT).context("eval/meeting_holdout.toml")
    }

    /// (who, what they say) in order.
    pub fn turns(&self) -> &[(String, String)] {
        &self.script.turns
    }
}

/// Where each turn ended up in the fixture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlacedTurn {
    pub who: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Truth {
    pub length_ms: u64,
    pub turns: Vec<PlacedTurn>,
}

#[derive(Debug, Default)]
pub struct ActionScore {
    pub id: String,
    pub found: bool,
    pub owner_right: bool,
    pub due_right: bool,
    /// What the best line said, to read in the report.
    pub line: Option<String>,
}

#[derive(Debug, Default)]
pub struct Score {
    pub decisions: Vec<(String, bool)>,
    pub actions: Vec<ActionScore>,
    /// Summary lines that match nothing known. Not all wrong: read them.
    pub extra_decisions: Vec<String>,
    pub extra_actions: Vec<String>,
    /// Decision lines, and the TL;DR, that still give an outcome the meeting took back.
    pub stale: Vec<String>,
}

impl Score {
    pub fn decisions_right(&self) -> usize {
        self.decisions.iter().filter(|(_, right)| *right).count()
    }

    pub fn actions_found(&self) -> usize {
        self.actions.iter().filter(|a| a.found).count()
    }

    /// Task, owner and due date all right.
    pub fn actions_right(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| a.found && a.owner_right && a.due_right)
            .count()
    }

    pub fn render(&self) -> String {
        let mut text = format!(
            "decisions right: {}/{}\n",
            self.decisions_right(),
            self.decisions.len()
        );
        for (id, right) in &self.decisions {
            let _ = writeln!(text, "  {} {id}", mark(*right));
        }
        let _ = writeln!(
            text,
            "action items found: {}/{}, fully right (owner and due too): {}/{}",
            self.actions_found(),
            self.actions.len(),
            self.actions_right(),
            self.actions.len()
        );
        for action in &self.actions {
            let _ = writeln!(
                text,
                "  {} {} (owner {}, due {}): {}",
                mark(action.found),
                action.id,
                mark(action.owner_right),
                mark(action.due_right),
                action.line.as_deref().unwrap_or("-")
            );
        }
        let _ = writeln!(text, "stale outcomes still shown: {}", self.stale.len());
        for line in &self.stale {
            let _ = writeln!(text, "  ! {line}");
        }
        let _ = writeln!(
            text,
            "lines matching nothing known: {}",
            self.extra_decisions.len() + self.extra_actions.len()
        );
        for line in self.extra_decisions.iter().chain(&self.extra_actions) {
            let _ = writeln!(text, "  ? {line}");
        }
        text
    }
}

fn mark(right: bool) -> &'static str {
    if right { "ok" } else { "--" }
}

fn matches_groups(line: &str, groups: &[Vec<String>]) -> bool {
    let line = line.to_lowercase();
    groups
        .iter()
        .all(|group| group.iter().any(|phrase| line.contains(phrase.as_str())))
}

pub fn score_summary(script: &MeetingScript, summary: &Summary) -> Score {
    let mut score = Score::default();
    for known in &script.decisions {
        let right = summary.decisions.iter().any(|item| {
            let line = item.text.to_lowercase();
            matches_groups(&line, &known.all)
                && !known.not.iter().any(|p| line.contains(p.as_str()))
        });
        score.decisions.push((known.id.clone(), right));
    }
    for known in &script.actions {
        let best = summary
            .action_items
            .iter()
            .filter(|item| matches_groups(&item.task, &known.task))
            .map(|item| {
                (
                    owner_right(script, known, item),
                    due_right(known, item),
                    item,
                )
            })
            .max_by_key(|(owner, due, _)| u8::from(*owner) + u8::from(*due));
        score.actions.push(match best {
            Some((owner_right, due_right, item)) => ActionScore {
                id: known.id.clone(),
                found: true,
                owner_right,
                due_right,
                line: Some(render_action(item)),
            },
            None => ActionScore {
                id: known.id.clone(),
                ..ActionScore::default()
            },
        });
    }
    let is_stale = |line: &str| {
        let line = line.to_lowercase();
        script
            .stale
            .iter()
            .any(|phrase| line.contains(phrase.as_str()))
    };
    score.stale = summary
        .decisions
        .iter()
        .map(|item| item.text.as_str())
        .chain(std::iter::once(summary.tldr.as_str()))
        .filter(|line| is_stale(line))
        .map(str::to_owned)
        .collect();
    score.extra_decisions = summary
        .decisions
        .iter()
        .filter(|item| {
            !script
                .decisions
                .iter()
                .any(|k| matches_groups(&item.text, &k.all))
        })
        .map(|item| item.text.clone())
        .collect();
    score.extra_actions = summary
        .action_items
        .iter()
        .filter(|item| {
            !script
                .actions
                .iter()
                .any(|k| matches_groups(&item.task, &k.task))
        })
        .map(|item| item.task.clone())
        .collect();
    score
}

fn render_action(item: &ActionItem) -> String {
    format!(
        "{} | {} | {}",
        item.owner.as_deref().unwrap_or("?"),
        item.task,
        item.due.as_deref().unwrap_or("?")
    )
}

fn owner_right(script: &MeetingScript, known: &KnownAction, item: &ActionItem) -> bool {
    item.owner.as_deref().is_some_and(|owner| {
        let lower = owner.to_lowercase();
        lower.contains(&known.owner.to_lowercase())
            || (known.owner == script.you && (lower == "you" || lower == "i"))
    })
}

fn due_right(known: &KnownAction, item: &ActionItem) -> bool {
    match item.due.as_deref() {
        None => known.due.is_empty(),
        Some(due) => {
            let due = due.to_lowercase();
            known.due.iter().any(|phrase| due.contains(phrase.as_str()))
        }
    }
}

/// Word-level edit distance over the reference's length, ignoring case and punctuation.
pub fn word_error_rate(reference: &str, heard: &str) -> f64 {
    let words = |text: &str| -> Vec<String> {
        text.split_whitespace()
            .map(|word| {
                word.chars()
                    .filter(|c| c.is_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect::<String>()
            })
            .filter(|word| !word.is_empty())
            .collect()
    };
    let (reference, heard) = (words(reference), words(heard));
    if reference.is_empty() {
        return if heard.is_empty() { 0.0 } else { 1.0 };
    }
    let mut row: Vec<usize> = (0..=heard.len()).collect();
    for (r, word) in reference.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = r + 1;
        for (h, other) in heard.iter().enumerate() {
            let substitute = diagonal + usize::from(word != other);
            diagonal = row[h + 1];
            row[h + 1] = substitute.min(row[h] + 1).min(row[h + 1] + 1);
        }
    }
    row[heard.len()] as f64 / reference.len() as f64
}
