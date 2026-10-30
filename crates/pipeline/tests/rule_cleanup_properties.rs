//! Properties of rule cleanup over generated, deliberately messy transcripts: fillers,
//! stutters, edit commands and correction markers mixed with content and punctuation.

use std::time::{Duration, Instant};

use wf_pipeline::rules::rule_cleanup;

const VOCABULARY: &[&str] = &[
    "the", "report", "we", "should", "ship", "it", "I", "think", "send", "to", "John", "Mary",
    "six", "seven", "at", "on", "Monday", "il", "progetto", "è", "pronto", "alle", "sei", "um",
    "uh", "ehm", "eh", "you", "know", "diciamo", "no", "wait", "sorry", "mean", "anzi", "scratch",
    "that", "delete", "never", "mind", "lascia", "perdere", "had", "piano", "così",
];
const PUNCTUATION: &[&str] = &["", "", "", "", ",", ",", ".", "?", "!"];
const LANGUAGES: &[Option<&str>] = &[Some("en"), Some("it"), None];

/// Deterministic xorshift so failures reproduce from the printed case.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }

    fn transcript(&mut self) -> String {
        let len = self.below(24);
        let mut words = Vec::new();
        for _ in 0..len {
            let word = VOCABULARY[self.below(VOCABULARY.len())];
            // Stutter now and then.
            let copies = if self.below(6) == 0 { 2 } else { 1 };
            for _ in 0..copies {
                words.push(format!(
                    "{word}{}",
                    PUNCTUATION[self.below(PUNCTUATION.len())]
                ));
            }
        }
        words.join(" ")
    }
}

fn cases() -> impl Iterator<Item = (Option<&'static str>, String)> {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    (0..5_000).map(move |_| {
        let language = LANGUAGES[rng.below(LANGUAGES.len())];
        (language, rng.transcript())
    })
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

#[test]
fn cleaning_twice_changes_nothing_more() {
    for (language, input) in cases() {
        let once = rule_cleanup(&input, language);
        let twice = rule_cleanup(&once, language);
        assert_eq!(once, twice, "[{language:?}] input {input:?}");
    }
}

#[test]
fn cleanup_only_deletes_words_never_adds_or_reorders_them() {
    for (language, input) in cases() {
        let output = words(&rule_cleanup(&input, language));
        let mut remaining = words(&input).into_iter();
        for word in &output {
            assert!(
                remaining.any(|w| &w == word),
                "[{language:?}] {word:?} is new or out of order\n input  {input:?}\n output {output:?}"
            );
        }
    }
}

#[test]
fn no_filler_survives() {
    for (language, input) in cases() {
        let output = rule_cleanup(&input, language);
        let fillers: &[&str] = match language {
            Some("en") => &["um", "uh"],
            Some("it") => &["ehm", "eh"],
            _ => &["um", "uh", "ehm", "eh"],
        };
        for filler in fillers {
            assert!(
                !words(&output).iter().any(|w| w == filler),
                "[{language:?}] {filler:?} left in {output:?} from {input:?}"
            );
        }
    }
}

#[test]
fn output_is_empty_or_a_capitalized_finished_sentence() {
    for (language, input) in cases() {
        let output = rule_cleanup(&input, language);
        if output.is_empty() {
            continue;
        }
        let first = output.chars().find(|c| c.is_alphanumeric()).unwrap();
        assert!(
            !first.is_lowercase(),
            "[{language:?}] {output:?} from {input:?}"
        );
        let mut tail = output.chars().rev();
        let (last, before_last) = (tail.next().unwrap(), tail.next());
        assert!(
            matches!(last, '.' | '?' | '!' | '…') && !matches!(before_last, Some(',' | ';' | ':')),
            "[{language:?}] {output:?} from {input:?}"
        );
    }
}

#[test]
fn cleans_500_words_within_the_5_ms_budget() {
    let mut rng = Rng(42);
    let mut input = String::new();
    while input.split_whitespace().count() < 500 {
        input.push_str(&rng.transcript());
        input.push(' ');
    }
    let fastest = (0..20)
        .map(|_| {
            let start = Instant::now();
            std::hint::black_box(rule_cleanup(std::hint::black_box(&input), Some("en")));
            start.elapsed()
        })
        .min()
        .unwrap();
    assert!(fastest < Duration::from_millis(5), "took {fastest:?}");
}
