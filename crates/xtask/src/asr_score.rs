//! The ASR benchmark's numbers: pooled WER, the time for a short clip and the real-time factor on
//! a long one, per language and kind of audio. `docs/benchmarks.md` takes its tables from here.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::speech_fixture::{Length, Source};

/// How the engine was told the language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// The app's default: "My languages" is empty.
    Auto,
    /// One language in "My languages".
    Pinned,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipResult {
    pub id: String,
    pub language: String,
    pub source: Source,
    pub length: Length,
    pub seconds: f32,
    pub mode: Mode,
    /// One per run.
    pub times: Vec<f64>,
    pub detected: Option<String>,
    pub text: String,
    /// True when a later run's text differed from the first.
    pub varied: bool,
    pub edits: usize,
    pub words: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BenchReport {
    pub model: String,
    pub cpu_threads: Option<usize>,
    pub load_seconds: f64,
    pub clips: Vec<ClipResult>,
}

/// One line of the summary: a language, a kind of audio, a way of choosing the language.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub language: String,
    pub source: Source,
    pub mode: Mode,
    pub clips: usize,
    pub wer: f64,
    /// Median over the short clips of each clip's median time, in seconds.
    pub short_seconds: Option<f64>,
    /// Processing time over audio time on the long clips.
    pub long_rtf: Option<f64>,
    pub wrong_language: usize,
}

pub fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    match sorted.len() {
        0 => 0.0,
        n if n % 2 == 1 => sorted[n / 2],
        n => f64::midpoint(sorted[n / 2 - 1], sorted[n / 2]),
    }
}

/// Groups results by language, audio and mode. WER is pooled: all edits over all reference words,
/// so a long clip counts for more than a short one.
pub fn summarize(results: &[ClipResult]) -> Vec<Row> {
    let mut keys: Vec<(&str, Source, Mode)> = results
        .iter()
        .map(|r| (r.language.as_str(), r.source, r.mode))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys.into_iter()
        .map(|(language, source, mode)| {
            let group: Vec<&ClipResult> = results
                .iter()
                .filter(|r| r.language == language && r.source == source && r.mode == mode)
                .collect();
            let edits: usize = group.iter().map(|r| r.edits).sum();
            let words: usize = group.iter().map(|r| r.words).sum();
            let short: Vec<f64> = group
                .iter()
                .filter(|r| r.length == Length::Short)
                .map(|r| median(&r.times))
                .collect();
            let long: Vec<&&ClipResult> =
                group.iter().filter(|r| r.length == Length::Long).collect();
            let long_time: f64 = long.iter().map(|r| median(&r.times)).sum();
            let long_audio: f64 = long.iter().map(|r| f64::from(r.seconds)).sum();
            Row {
                language: language.to_owned(),
                source,
                mode,
                clips: group.len(),
                wer: edits as f64 / words.max(1) as f64,
                short_seconds: (!short.is_empty()).then(|| median(&short)),
                long_rtf: (!long.is_empty()).then(|| long_time / long_audio),
                wrong_language: group
                    .iter()
                    .filter(|r| r.detected.as_deref().is_some_and(|d| d != r.language))
                    .count(),
            }
        })
        .collect()
}

pub fn render(report: &BenchReport) -> String {
    let mut out = format!(
        "{}{}: loaded in {:.2} s\n",
        report.model,
        report
            .cpu_threads
            .map_or(String::new(), |n| format!(" on {n} CPU threads")),
        report.load_seconds,
    );
    let number = |value: Option<f64>, digits: usize| {
        value.map_or("-".to_owned(), |value| format!("{value:.digits$}"))
    };
    for row in summarize(&report.clips) {
        let _ = writeln!(
            out,
            "{} {:9} {:6} {:3} clips  WER {:5.1}%  short {:>6} s  long RTF {:>6}  wrong language {}",
            row.language,
            format!("{:?}", row.source).to_lowercase(),
            format!("{:?}", row.mode).to_lowercase(),
            row.clips,
            row.wer * 100.0,
            number(row.short_seconds, 3),
            number(row.long_rtf, 3),
            row.wrong_language,
        );
    }
    let varied = report.clips.iter().filter(|r| r.varied).count();
    let _ = writeln!(out, "clips whose text changed between runs: {varied}");
    out
}
