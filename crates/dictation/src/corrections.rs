//! Learning a word from the user's own fix (spec §5). After an insertion this looks at the field
//! for a short while, and a word goes to the dictionary only if the text we inserted is still
//! there with one word changed, a little, and that change holds still. Anything else is ignored: a
//! wrong word in the dictionary skews every later dictation, and a missed one costs a retype.
//!
//! Only the new spelling leaves this module. The text being compared lives in memory for the
//! length of the watch.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use wf_platform::context::{Context, ContextDepth, ContextSource, Focus};

use crate::Event;
use crate::coordinator::Input;

/// How often the field is read during a watch.
pub const LOOK_EVERY: Duration = Duration::from_secs(1);
/// How long after an insertion a fix still counts as one.
pub const WATCH_FOR: Duration = Duration::from_secs(30);
/// Looks in a row a fix must stay the same, so a word being typed isn't taken for the result.
pub const STEADY_LOOKS: u32 = 4;
/// One word needs another beside it to tell where it belongs.
const MIN_WORDS: usize = 2;
/// Shorter words are grammar more often than spelling: "to" and "too", "its" and "it's".
const MIN_CHARS: usize = 4;
const MAX_CHARS: usize = 32;
const MAX_EDITS: usize = 3;

pub(crate) enum Job {
    Begin(Watch),
    Look,
    End,
}

/// The text of one insertion, as we wrote it.
pub(crate) struct Watch {
    id: u64,
    app: String,
    words: Vec<String>,
    /// The text showed up in the field unchanged, which proves this field holds it.
    seen: bool,
    /// The fix on screen, and how many looks in a row it has held.
    steady: Option<(String, u32)>,
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Keep,
    Stop,
    Learn(String),
}

impl Watch {
    pub(crate) fn new(id: u64, text: &str, app: Option<String>) -> Option<Self> {
        let words: Vec<String> = text.split_whitespace().map(str::to_owned).collect();
        Some(Self {
            id,
            app: app?,
            seen: false,
            steady: None,
            words: (words.len() >= MIN_WORDS).then_some(words)?,
        })
    }

    fn look(&mut self, context: &Context) -> Outcome {
        let app = context.app.as_ref().map(|app| app.id.as_str());
        if context.focus == Focus::Secure || app != Some(&self.app) {
            return Outcome::Stop;
        }
        let around = context.text.as_ref();
        let Some(around) = around.filter(|_| context.focus == Focus::Editable) else {
            self.steady = None;
            return Outcome::Keep;
        };
        let field = format!("{}{}{}", around.before, around.selection, around.after);
        let field: Vec<&str> = field.split_whitespace().collect();
        let fix = match difference(&self.words, &field) {
            Difference::None => {
                self.seen = true;
                None
            }
            Difference::OneWord(index, shown) if self.seen => self.fix(index, shown),
            Difference::OneWord(..) | Difference::More => None,
        };
        let Some(word) = fix else {
            self.steady = None;
            return Outcome::Keep;
        };
        let looks = match &self.steady {
            Some((steady, looks)) if *steady == word => looks + 1,
            _ => 1,
        };
        if looks >= STEADY_LOOKS {
            return Outcome::Learn(word);
        }
        self.steady = Some((word, looks));
        Outcome::Keep
    }

    fn fix(&self, index: usize, shown: &str) -> Option<String> {
        let opens_sentence = match index.checked_sub(1) {
            Some(before) => self.words[before].ends_with(['.', '!', '?', ':']),
            None => true,
        };
        learned_word(&self.words[index], shown, opens_sentence)
    }
}

enum Difference<'a> {
    None,
    /// Which word of ours, and what the field has there.
    OneWord(usize, &'a str),
    More,
}

/// Finds our words in the field by sliding them along it, so the text around them doesn't matter.
/// A word added, removed or split throws everything after it out of line, which reads as `More`.
/// So do two places that fit equally well.
fn difference<'a>(words: &[String], field: &[&'a str]) -> Difference<'a> {
    let mut fixes = field.windows(words.len()).filter_map(|window| {
        let mut differing = (0..words.len()).filter(|&at| window[at] != words[at]);
        match (differing.next(), differing.next()) {
            (None, _) => Some(None),
            (Some(at), None) => Some(Some((at, window[at]))),
            _ => None,
        }
    });
    match (fixes.next(), fixes.next()) {
        (Some(None), _) => Difference::None,
        (_, Some(None)) => Difference::None,
        (Some(Some((at, shown))), None) => Difference::OneWord(at, shown),
        _ => Difference::More,
    }
}

/// The word to keep from a fix of `old` to `new`, if it looks like a change of spelling.
fn learned_word(old: &str, new: &str, opens_sentence: bool) -> Option<String> {
    let (old_open, old, old_close) = split_marks(old)?;
    let (new_open, new, new_close) = split_marks(new)?;
    if old_open != new_open || old_close != new_close || !is_word(old) || !is_word(new) {
        return None;
    }
    let (old_chars, new_chars) = (old.chars().count(), new.chars().count());
    if old_chars.min(new_chars) < MIN_CHARS || old_chars.max(new_chars) > MAX_CHARS {
        return None;
    }
    // A changed number is a changed fact.
    if digits(old).ne(digits(new)) {
        return None;
    }
    let (old_lower, new_lower) = (old.to_lowercase(), new.to_lowercase());
    if old_lower == new_lower {
        // "GitHub" spells a name. "Hello" and "NASA" could be style or emphasis.
        return has_inner_capital(new).then(|| new.to_owned());
    }
    // A word cut short is a word being retyped.
    if old_lower.starts_with(&new_lower) {
        return None;
    }
    if edits(&old_lower, &new_lower) > (old_chars.max(new_chars) / 3).clamp(1, MAX_EDITS) {
        return None;
    }
    // A sentence's first letter is a capital anyway, so it says nothing about the word.
    if opens_sentence && new.starts_with(char::is_uppercase) && !has_inner_capital(new) {
        return None;
    }
    Some(new.to_owned())
}

/// Splits punctuation off both ends of a token.
fn split_marks(token: &str) -> Option<(&str, &str, &str)> {
    let mark = |c: char| !c.is_alphanumeric();
    let end = token.trim_end_matches(mark).len();
    let start = end - token[..end].trim_start_matches(mark).len();
    (start < end).then(|| (&token[..start], &token[start..end], &token[end..]))
}

/// Letters and digits, perhaps hyphenated. Rules out mail addresses, paths and contractions.
fn is_word(word: &str) -> bool {
    word.chars().all(|c| c.is_alphanumeric() || c == '-') && word.chars().any(char::is_alphabetic)
}

fn digits(word: &str) -> impl Iterator<Item = char> {
    word.chars().filter(char::is_ascii_digit)
}

fn has_inner_capital(word: &str) -> bool {
    word.chars().skip(1).any(char::is_uppercase) && word.chars().any(char::is_lowercase)
}

/// Levenshtein distance. A swap of two letters counts as two, which keeps out "form" for "from".
fn edits(from: &str, to: &str) -> usize {
    let to: Vec<char> = to.chars().collect();
    let mut row: Vec<usize> = (0..=to.len()).collect();
    for (i, a) in from.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, &b) in to.iter().enumerate() {
            let replaced = diagonal + usize::from(a != b);
            diagonal = row[j + 1];
            row[j + 1] = replaced.min(row[j] + 1).min(diagonal + 1);
        }
    }
    row[to.len()]
}

/// Runs the watches. Reads can stall on a busy app, so a read covers all the looks that piled up
/// behind it.
pub(crate) fn run(
    jobs: Receiver<Job>,
    context: Arc<dyn ContextSource + Send + Sync>,
    inbox: Sender<Input>,
    on_event: Arc<dyn Fn(Event) + Send + Sync>,
) {
    let mut watch: Option<Watch> = None;
    while let Ok(job) = jobs.recv() {
        let mut look = false;
        for job in std::iter::once(job).chain(jobs.try_iter()) {
            match job {
                Job::Begin(new) => watch = Some(new),
                Job::Look => look = true,
                Job::End => watch = None,
            }
        }
        let Some(current) = watch.as_mut().filter(|_| look) else {
            continue;
        };
        let outcome = current.look(&context.read(ContextDepth::WithText));
        if outcome == Outcome::Keep {
            continue;
        }
        let _ = inbox.send(Input::WatchOver(current.id));
        watch = None;
        if let Outcome::Learn(word) = outcome {
            on_event(Event::WordLearned(word));
        }
    }
}

#[cfg(test)]
mod tests {
    use wf_platform::context::{AppIdentity, TextAround};

    use super::*;

    const APP: &str = "com.example.Notes";

    fn watch(text: &str) -> Watch {
        Watch::new(1, text, Some(APP.to_owned())).unwrap()
    }

    fn field(text: &str) -> Context {
        Context {
            app: Some(AppIdentity {
                id: APP.to_owned(),
                name: None,
                pid: None,
            }),
            window_title: None,
            focus: Focus::Editable,
            text: Some(TextAround::new(text, "", "")),
        }
    }

    /// Looks at `then` until it settles, after the original text was seen.
    fn after_fix(text: &str, then: &str) -> Outcome {
        let mut watch = watch(text);
        assert_eq!(watch.look(&field(text)), Outcome::Keep);
        let mut outcome = Outcome::Keep;
        for _ in 0..STEADY_LOOKS {
            outcome = watch.look(&field(then));
        }
        outcome
    }

    fn learns(text: &str, then: &str, word: &str) {
        assert_eq!(after_fix(text, then), Outcome::Learn(word.to_owned()));
    }

    fn ignores(text: &str, then: &str) {
        assert_eq!(after_fix(text, then), Outcome::Keep, "{then}");
    }

    #[test]
    fn a_small_fix_to_one_word_is_learned() {
        learns(
            "We deploy on kubernets today.",
            "We deploy on kubernetes today.",
            "kubernetes",
        );
        learns("Ask Frederico first.", "Ask Federico first.", "Federico");
        learns("We use Wisper here.", "We use Wispr here.", "Wispr");
        learns("Il perche non lo so.", "Il perché non lo so.", "perché");
    }

    #[test]
    fn punctuation_around_the_word_stays_out_of_it() {
        learns(
            "Then (kubernets), maybe.",
            "Then (kubernetes), maybe.",
            "kubernetes",
        );
        learns("We run kubernets.", "We run kubernetes.", "kubernetes");
    }

    #[test]
    fn the_first_word_and_the_last_word_can_be_fixed() {
        learns("kubernets is fine", "kubernetes is fine", "kubernetes");
        learns("this is kubernets", "this is kubernetes", "kubernetes");
    }

    #[test]
    fn a_capital_inside_a_word_is_a_spelling() {
        learns("We host on github now.", "We host on GitHub now.", "GitHub");
        learns(
            "I bought an iphone today.",
            "I bought an iPhone today.",
            "iPhone",
        );
    }

    #[test]
    fn a_change_of_case_alone_is_style() {
        ignores("the internet is down", "the Internet is down");
        ignores("the nasa launch was good", "the NASA launch was good");
        ignores("this is very important", "this is very IMPORTANT");
        ignores("Hello there friend", "hello there friend");
    }

    #[test]
    fn the_capital_that_opens_a_sentence_says_nothing() {
        ignores("Frederico is here.", "Federico is here.");
        ignores(
            "All done. Frederico is here.",
            "All done. Federico is here.",
        );
        learns("Ask Frederico first.", "Ask Federico first.", "Federico");
    }

    #[test]
    fn a_small_word_is_grammar_more_often_than_spelling() {
        ignores("I went too far", "I went to far");
        ignores("it was the end", "it is the end");
        ignores("the cat sat here", "the car sat here");
    }

    #[test]
    fn a_different_word_is_a_change_of_mind() {
        ignores("I like the red shirt", "I like the blue shirt");
        ignores("meet me on monday please", "meet me on tuesday please");
        ignores("we need seven chairs", "we need eight chairs");
        ignores("please fill the form", "please fill the from");
    }

    #[test]
    fn a_changed_number_is_not_a_spelling() {
        ignores("the room is 1204 today", "the room is 1205 today");
        ignores("use model v2beta here", "use model v3beta here");
    }

    #[test]
    fn a_word_cut_short_is_still_being_typed() {
        ignores("we use kubernets here", "we use kubern here");
        ignores("we use helo here", "we use hel here");
    }

    #[test]
    fn contractions_addresses_and_paths_are_not_words() {
        ignores("dont forget this", "don't forget this");
        ignores(
            "write to anna@example.com soon",
            "write to anna@example.org soon",
        );
        ignores("look in src/main today", "look in src/mains today");
    }

    #[test]
    fn a_word_changed_along_with_its_punctuation_is_more_than_a_fix() {
        ignores("we run kubernets.", "we run kubernetes!");
        ignores("a kubernets cluster", "a (kubernetes) cluster");
    }

    #[test]
    fn only_one_word_may_change() {
        ignores(
            "we deploy kubernets to prodution",
            "we deploy kubernetes to production",
        );
    }

    #[test]
    fn a_rewrite_is_not_a_fix() {
        ignores(
            "We deploy on kubernets today.",
            "Honestly, the deploy went badly and I am going home.",
        );
        ignores("We deploy on kubernets today.", "");
    }

    #[test]
    fn a_word_added_or_split_or_merged_is_not_a_fix() {
        ignores("we use the airplane here", "we use the air plane here");
        ignores("we use the data base here", "we use the database here");
        ignores("we use this here", "we use all of this here");
    }

    #[test]
    fn the_text_around_ours_does_not_matter() {
        learns(
            "We deploy on kubernets today.",
            "Notes from Tuesday. We deploy on kubernetes today. See you there.",
            "kubernetes",
        );
    }

    #[test]
    fn the_cursor_may_be_anywhere() {
        let text = "We deploy on kubernets today.";
        let mut watch = watch(text);
        watch.look(&field(text));
        let mut split = field("");
        split.text = Some(TextAround::new("We deploy on kuber", "net", "es today."));

        for _ in 1..STEADY_LOOKS {
            assert_eq!(watch.look(&split), Outcome::Keep);
        }
        assert_eq!(watch.look(&split), Outcome::Learn("kubernetes".to_owned()));
    }

    #[test]
    fn a_text_that_was_never_seen_whole_teaches_nothing() {
        let mut watch = watch("We deploy on kubernets today.");

        for _ in 0..STEADY_LOOKS * 2 {
            let shown = field("We deploy on kubernetes today.");
            assert_eq!(watch.look(&shown), Outcome::Keep);
        }
    }

    #[test]
    fn a_word_must_hold_still() {
        let text = "We deploy on kubernets today.";
        let mut watch = watch(text);
        watch.look(&field(text));

        for typed in ["kubernete", "kubernetess", "kubernetes"] {
            let shown = field(&format!("We deploy on {typed} today."));
            for _ in 1..STEADY_LOOKS {
                assert_eq!(watch.look(&shown), Outcome::Keep, "{typed}");
            }
        }
        let shown = field("We deploy on kubernetes today.");
        assert_eq!(watch.look(&shown), Outcome::Learn("kubernetes".to_owned()));
    }

    #[test]
    fn a_fix_that_is_undone_starts_over() {
        let text = "We deploy on kubernets today.";
        let fixed = field("We deploy on kubernetes today.");
        let mut watch = watch(text);
        watch.look(&field(text));

        for _ in 1..STEADY_LOOKS {
            watch.look(&fixed);
        }
        watch.look(&field(text));

        for _ in 1..STEADY_LOOKS {
            assert_eq!(watch.look(&fixed), Outcome::Keep);
        }
    }

    #[test]
    fn two_equally_good_places_are_left_alone() {
        ignores(
            "see the kubernets docs",
            "see the kubernetes docs and see the kubernetees docs",
        );
    }

    #[test]
    fn a_secure_field_ends_the_watch_whatever_it_shows() {
        let text = "We deploy on kubernets today.";
        let mut watch = watch(text);
        watch.look(&field(text));
        let mut secure = field("We deploy on kubernetes today.");
        secure.focus = Focus::Secure;

        assert_eq!(watch.look(&secure), Outcome::Stop);
    }

    #[test]
    fn another_app_ends_the_watch() {
        let text = "We deploy on kubernets today.";
        let mut watch = watch(text);
        let mut other = field(text);
        other.app.as_mut().unwrap().id = "com.example.Other".to_owned();

        assert_eq!(watch.look(&other), Outcome::Stop);
    }

    #[test]
    fn only_a_real_text_field_counts() {
        let text = "We deploy on kubernets today.";
        let mut watch = watch(text);
        watch.look(&field(text));
        let mut unknown = field("We deploy on kubernetes today.");
        unknown.focus = Focus::Unknown;

        for _ in 0..STEADY_LOOKS {
            assert_eq!(watch.look(&unknown), Outcome::Keep);
        }
    }

    #[test]
    fn one_word_alone_has_nothing_to_line_up_with() {
        assert!(Watch::new(1, "Kubernets", Some(APP.to_owned())).is_none());
    }

    #[test]
    fn no_app_means_no_watch() {
        assert!(Watch::new(1, "two words", None).is_none());
    }

    #[test]
    fn edits_count_single_letters() {
        assert_eq!(edits("kitten", "sitting"), 3);
        assert_eq!(edits("form", "from"), 2);
        assert_eq!(edits("", "abc"), 3);
    }
}
