//! The LLM cleanup eval (ticket 09). Each case runs the way a dictation does, through rule
//! cleanup, LLM cleanup with its guards and timeout, and the local passes, so a timeout or a
//! rejected reply scores whatever the user would have received.

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use serde::Deserialize;
use wf_pipeline::cleanup::{CleanupRequest, DEFAULT_TIMEOUT, llm_cleanup};
use wf_pipeline::passes::{AppCategory, LocalPassOptions, Style, local_passes};
use wf_pipeline::rules::rule_cleanup;
use wf_pipeline::{Llm, LlmPrompt, PipelineError};

const CASES: &str = include_str!("../eval/cleanup.toml");

/// The ticket's bar on Balanced.
pub const TARGET: f64 = 0.90;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub category: String,
    pub language: Option<String>,
    pub input: String,
    pub expect: Vec<String>,
    #[serde(default)]
    pub contains: Vec<String>,
    #[serde(default)]
    pub absent: Vec<String>,
    pub list_items: Option<usize>,
    #[serde(default)]
    pub lowercase: bool,
    pub final_period: Option<bool>,
    pub style: Option<StyleName>,
    #[serde(default)]
    pub email: bool,
    #[serde(default)]
    pub dictionary: Vec<String>,
    /// Kept out of prompt tuning, so the final score has cases the prompt never saw.
    #[serde(default)]
    pub holdout: bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StyleName {
    Formal,
    Casual,
    VeryCasual,
    Excited,
}

impl From<StyleName> for Style {
    fn from(name: StyleName) -> Self {
        match name {
            StyleName::Formal => Style::Formal,
            StyleName::Casual => Style::Casual,
            StyleName::VeryCasual => Style::VeryCasual,
            StyleName::Excited => Style::Excited,
        }
    }
}

pub fn cases() -> anyhow::Result<Vec<Case>> {
    #[derive(Deserialize)]
    struct File {
        case: Vec<Case>,
    }
    let file: File = toml::from_str(CASES).context("parsing eval/cleanup.toml")?;
    Ok(file.case)
}

/// Returns what is wrong with `output`; empty means the case passes.
pub fn score(case: &Case, output: &str) -> Vec<String> {
    let mut misses = Vec::new();
    let words = words(output);
    let matches_reference = case.expect.iter().any(|reference| {
        let reference = self::words(reference);
        word_distance(&words, &reference) <= reference.len() / 10
    });
    if !matches_reference {
        misses.push("no reference matches".to_owned());
    }
    for needle in &case.contains {
        if !output.contains(needle.as_str()) {
            misses.push(format!("missing {needle:?}"));
        }
    }
    for needle in &case.absent {
        if output.contains(needle.as_str()) {
            misses.push(format!("should not contain {needle:?}"));
        }
    }

    let items = output.lines().filter(|l| strip_marker(l).is_some()).count();
    if let Some(wanted) = case.list_items
        && items != wanted
    {
        misses.push(format!("{items} list items, wanted {wanted}"));
    }

    if case.lowercase {
        if output != output.to_lowercase() {
            misses.push("should be all lowercase".to_owned());
        }
    } else if !output
        .chars()
        .find(|c| c.is_alphanumeric())
        .is_some_and(|c| !c.is_lowercase())
    {
        misses.push("should start capitalized".to_owned());
    }

    let ends_a_sentence = output.ends_with(['.', '!', '?']);
    match case.final_period {
        Some(false) if output.ends_with('.') => misses.push("should drop the final period".into()),
        None if items == 0 && !ends_a_sentence => misses.push("should end a sentence".into()),
        _ => {}
    }
    misses
}

/// Compared words: lowercase, without list markers or the punctuation around them.
fn words(text: &str) -> Vec<String> {
    text.lines()
        .flat_map(|line| strip_marker(line).unwrap_or(line).split_whitespace())
        .map(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

/// The text after a list marker ("- ", "* ", "• ", "1. ", "1) "), if the line has one.
fn strip_marker(line: &str) -> Option<&str> {
    let line = line.trim_start();
    if let Some(rest) = ["- ", "* ", "• "]
        .iter()
        .find_map(|marker| line.strip_prefix(marker))
    {
        return Some(rest);
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    (digits > 0)
        .then(|| &line[digits..])
        .and_then(|rest| rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")))
}

fn word_distance(a: &[String], b: &[String]) -> usize {
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, word) in a.iter().enumerate() {
        let mut current = vec![i + 1];
        for (j, other) in b.iter().enumerate() {
            let substitution = previous[j] + usize::from(word != other);
            current.push(substitution.min(previous[j + 1] + 1).min(current[j] + 1));
        }
        previous = current;
    }
    previous[b.len()]
}

#[derive(Debug)]
pub struct CaseResult {
    pub id: String,
    pub category: String,
    pub language: Option<String>,
    pub holdout: bool,
    pub input: String,
    pub output: String,
    pub misses: Vec<String>,
    pub fallback: Option<String>,
    pub elapsed: Duration,
}

#[derive(Debug, Default)]
pub struct Report {
    pub results: Vec<CaseResult>,
}

pub fn run(cases: &[Case], llm: &mut dyn Llm) -> Report {
    let mut report = Report::default();
    for case in cases {
        let cleaned = rule_cleanup(&case.input, case.language.as_deref());
        let request = CleanupRequest {
            text: &cleaned,
            language: case.language.as_deref(),
            style: case.style.map(Style::from),
            dictionary: &case.dictionary,
            ..CleanupRequest::default()
        };
        let mut recorder = Recorder {
            llm: &mut *llm,
            reply: None,
        };
        let started = Instant::now();
        let outcome = llm_cleanup(&mut recorder, &request, DEFAULT_TIMEOUT);
        let elapsed = started.elapsed();
        let fallback = outcome
            .fallback
            .map(|fallback| match recorder.reply.take() {
                Some(reply) => format!("{fallback:?}, model said {reply:?}"),
                None => format!("{fallback:?}"),
            });
        let output = local_passes(
            &outcome.text,
            &LocalPassOptions {
                // Formal leaves the text as it is, so it stands in for "no style".
                style: case.style.map_or(Style::Formal, Style::from),
                category: if case.email {
                    AppCategory::Email
                } else {
                    AppCategory::Other
                },
                snippets: &[],
                replacements: &[],
                press_enter_command: false,
                before: None,
                after: None,
            },
        )
        .text;
        report.results.push(CaseResult {
            id: case.id.clone(),
            category: case.category.clone(),
            language: case.language.clone(),
            holdout: case.holdout,
            input: cleaned.clone(),
            misses: score(case, &output),
            fallback,
            output,
            elapsed,
        });
    }
    report
}

/// Keeps the model's reply so the report can show what a guard threw out.
struct Recorder<'a> {
    llm: &'a mut dyn Llm,
    reply: Option<String>,
}

impl Llm for Recorder<'_> {
    fn generate(
        &mut self,
        prompt: &LlmPrompt,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<String, PipelineError> {
        let reply = self.llm.generate(prompt, on_token)?;
        self.reply = Some(reply.clone());
        Ok(reply)
    }
}

/// A model that always fails, which leaves the rule-cleaned text: the score without an LLM.
pub struct RulesOnly;

impl Llm for RulesOnly {
    fn generate(
        &mut self,
        _prompt: &LlmPrompt,
        _on_token: &mut dyn FnMut(&str),
    ) -> Result<String, PipelineError> {
        Err(PipelineError::Engine("rules only".into()))
    }
}

impl Report {
    pub fn passed(&self) -> usize {
        self.results.iter().filter(|r| r.misses.is_empty()).count()
    }

    pub fn score(&self) -> f64 {
        self.passed() as f64 / self.results.len().max(1) as f64
    }

    pub fn failures(&self) -> impl Iterator<Item = &CaseResult> {
        self.results.iter().filter(|r| !r.misses.is_empty())
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        for result in self.failures() {
            let _ = writeln!(out, "FAIL {} [{}]", result.id, result.misses.join("; "));
            let _ = writeln!(out, "  in:  {:?}", result.input);
            let _ = writeln!(out, "  out: {:?}", result.output);
            if let Some(fallback) = &result.fallback {
                let _ = writeln!(out, "  fallback: {fallback}");
            }
        }

        let tally = |keep: &dyn Fn(&CaseResult) -> bool| {
            let kept: Vec<_> = self.results.iter().filter(|r| keep(r)).collect();
            let passed = kept.iter().filter(|r| r.misses.is_empty()).count();
            format!("{passed}/{}", kept.len())
        };
        let mut categories: Vec<&str> = self.results.iter().map(|r| r.category.as_str()).collect();
        categories.sort_unstable();
        categories.dedup();
        let _ = writeln!(out);
        for category in categories {
            let line = tally(&|r: &CaseResult| r.category == category);
            let _ = writeln!(out, "{category:<16} {line}");
        }
        let _ = writeln!(
            out,
            "{:<16} {}",
            "tuned",
            tally(&|r: &CaseResult| !r.holdout)
        );
        let _ = writeln!(
            out,
            "{:<16} {}",
            "holdout",
            tally(&|r: &CaseResult| r.holdout)
        );

        let mut languages: Vec<&str> = self
            .results
            .iter()
            .filter_map(|r| r.language.as_deref())
            .collect();
        languages.sort_unstable();
        languages.dedup();
        for language in languages {
            let line = tally(&|r: &CaseResult| r.language.as_deref() == Some(language));
            let mut times: Vec<_> = self
                .results
                .iter()
                .filter(|r| r.language.as_deref() == Some(language))
                .map(|r| r.elapsed)
                .collect();
            times.sort_unstable();
            let _ = writeln!(
                out,
                "{:<16} {line}, p50 {:?}",
                format!("language {language}"),
                times[times.len() / 2],
            );
        }

        let mut elapsed: Vec<_> = self.results.iter().map(|r| r.elapsed).collect();
        elapsed.sort_unstable();
        let percentile = |p: usize| elapsed.get(elapsed.len() * p / 100).copied();
        let fallbacks = self.results.iter().filter(|r| r.fallback.is_some()).count();
        let _ = writeln!(
            out,
            "fallbacks        {fallbacks}\nlatency          p50 {:?}, p95 {:?}, max {:?}",
            percentile(50).unwrap_or_default(),
            percentile(95).unwrap_or_default(),
            elapsed.last().copied().unwrap_or_default(),
        );
        let _ = writeln!(
            out,
            "score            {}/{} = {:.1}% (target {:.0}%)",
            self.passed(),
            self.results.len(),
            self.score() * 100.0,
            TARGET * 100.0,
        );
        out
    }
}
