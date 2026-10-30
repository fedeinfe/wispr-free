//! Local passes (spec §3): the deterministic steps between cleanup and insertion. Each pass is a
//! pure function, and [`local_passes`] runs them in order for one dictation.

use crate::rules::capitalize;

/// Tone and punctuation preset for an app category. LLM cleanup applies a style by rewriting;
/// [`apply_style`] does the mechanical part, so the style holds with the LLM off too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Formal,
    Casual,
    VeryCasual,
    Excited,
}

/// What the focused app is for, which picks its style (spec §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppCategory {
    PersonalMessages,
    WorkMessages,
    Email,
    Other,
}

/// A user snippet: saying `trigger` writes `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    pub trigger: String,
    pub text: String,
}

/// A dictionary replacement: `wrong` is always written as `right`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    pub wrong: String,
    pub right: String,
}

/// Everything a dictation's local passes depend on.
#[derive(Debug, Clone, Copy)]
pub struct LocalPassOptions<'a> {
    pub style: Style,
    pub category: AppCategory,
    pub snippets: &'a [Snippet],
    pub replacements: &'a [Replacement],
    /// The user turned on the "press enter" voice command.
    pub press_enter_command: bool,
    /// The text before the cursor. `None` when context is off or unreadable.
    pub before: Option<&'a str>,
    /// The text after the cursor. `None` when context is off or unreadable.
    pub after: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalPassOutput {
    /// What to insert. Empty when the dictation was only the press-enter command.
    pub text: String,
    /// Press Enter after inserting.
    pub press_enter: bool,
}

/// Runs the local passes for one dictation. The order is what keeps user text exact: the style
/// and casing passes run before the dictionary and snippets, so they can't undo a spelling, and
/// a dictation that is only a trigger skips everything that would edit its expansion.
pub fn local_passes(text: &str, options: &LocalPassOptions) -> LocalPassOutput {
    let stripped = options
        .press_enter_command
        .then(|| strip_press_enter(text))
        .flatten();
    let press_enter = stripped.is_some();
    let text = stripped.as_deref().unwrap_or(text);
    let written = match whole_snippet(text, options.snippets) {
        Some(expansion) => expansion.to_owned(),
        None => {
            let mut styled = apply_style_for(text, options);
            styled = match_casing(&styled, options.before);
            styled = apply_replacements(&styled, options.replacements);
            expand_snippets(&styled, options.snippets)
        }
    };
    LocalPassOutput {
        text: match_spacing(&written, options.before, options.after),
        press_enter,
    }
}

fn apply_style_for(text: &str, options: &LocalPassOptions) -> String {
    match options.category {
        AppCategory::Email => apply_style(&break_after_greeting(text), options.style),
        AppCategory::PersonalMessages | AppCategory::WorkMessages => {
            drop_chat_period(&apply_style(text, options.style))
        }
        AppCategory::Other => apply_style(text, options.style),
    }
}

/// Lowercase pieces of app ids, by what the app is for.
const APPS: &[(&str, AppCategory)] = &[
    ("slack", AppCategory::WorkMessages),
    ("teams", AppCategory::WorkMessages),
    ("mattermost", AppCategory::WorkMessages),
    ("zulip", AppCategory::WorkMessages),
    ("webex", AppCategory::WorkMessages),
    ("whatsapp", AppCategory::PersonalMessages),
    ("telegram", AppCategory::PersonalMessages),
    ("mobilesms", AppCategory::PersonalMessages),
    ("signal", AppCategory::PersonalMessages),
    ("discord", AppCategory::PersonalMessages),
    ("messenger", AppCategory::PersonalMessages),
    ("mail", AppCategory::Email),
    ("outlook", AppCategory::Email),
    ("thunderbird", AppCategory::Email),
    ("superhuman", AppCategory::Email),
];

/// Lowercase web app names as they show up in a tab title.
const WEB_APPS: &[(&str, AppCategory)] = &[
    ("gmail", AppCategory::Email),
    ("outlook", AppCategory::Email),
    ("slack", AppCategory::WorkMessages),
    ("microsoft teams", AppCategory::WorkMessages),
    ("whatsapp", AppCategory::PersonalMessages),
    ("telegram", AppCategory::PersonalMessages),
    ("discord", AppCategory::PersonalMessages),
    ("messenger", AppCategory::PersonalMessages),
];

/// Lowercase pieces of browser app ids.
const BROWSERS: &[&str] = &[
    "chrome",
    "chromium",
    "firefox",
    "safari",
    "edge",
    "brave",
    "vivaldi",
    "opera",
    "thebrowser",
];

/// The category of the focused app, from the context's app id and window title.
pub fn app_category(app_id: Option<&str>, window_title: Option<&str>) -> AppCategory {
    let id = app_id.unwrap_or_default().to_lowercase();
    let lookup = |table: &[(&str, AppCategory)], name: &str| {
        let hit = table.iter().find(|(piece, _)| name.contains(piece));
        hit.map(|&(_, category)| category)
    };
    if BROWSERS.iter().any(|browser| id.contains(browser)) {
        let title = window_title.unwrap_or_default().to_lowercase();
        return lookup(WEB_APPS, &title).unwrap_or(AppCategory::Other);
    }
    lookup(APPS, &id).unwrap_or(AppCategory::Other)
}

/// Coordinating conjunctions (en, it) a Casual message needs no comma before. Other commas stay,
/// since they can carry meaning.
const SOFT_COMMA_WORDS: &[&str] = &["and", "but", "or", "so", "e", "ma", "o", "però"];

/// Rewrites the sentence case and final period rule cleanup always emits. Only the final period
/// changes: an inner one may belong to an abbreviation. Idempotent.
pub fn apply_style(text: &str, style: Style) -> String {
    match style {
        Style::Formal => text.to_owned(),
        Style::Casual => {
            let relaxed = SOFT_COMMA_WORDS.iter().fold(text.to_owned(), |text, word| {
                text.replace(&format!(", {word} "), &format!(" {word} "))
            });
            without_final_period(&relaxed)
                .unwrap_or(&relaxed)
                .to_owned()
        }
        Style::VeryCasual => apply_style(text, Style::Casual).to_lowercase(),
        Style::Excited => match without_final_period(text) {
            Some(rest) => format!("{rest}!"),
            None => text.to_owned(),
        },
    }
}

/// The text before its final period. An ellipsis is not a final period.
fn without_final_period(text: &str) -> Option<&str> {
    text.strip_suffix('.').filter(|rest| !rest.ends_with('.'))
}

/// Words that open a greeting (en, it). Lowercase.
const GREETINGS: &[&str] = &[
    "hi",
    "hello",
    "hey",
    "dear",
    "good morning",
    "good afternoon",
    "good evening",
    "ciao",
    "salve",
    "buongiorno",
    "buonasera",
    "caro",
    "cara",
    "cari",
    "care",
    "gentile",
    "gentili",
    "egregio",
    "egregia",
];

/// How far into the text a greeting may run before its comma stops being the greeting's.
const MAX_GREETING_WORDS: usize = 6;

/// Email only: moves what follows a greeting onto its own line.
pub fn break_after_greeting(text: &str) -> String {
    let greeted = GREETINGS.iter().any(|g| phrase_len(text, g).is_some());
    let end = text
        .find([',', ':', '!'])
        .filter(|&end| greeted && text[..end].split_whitespace().count() <= MAX_GREETING_WORDS);
    let Some(end) = end else {
        return text.to_owned();
    };
    let (greeting, rest) = text.split_at(end + 1);
    match rest.strip_prefix(' ').map(str::trim_start) {
        Some(body) if !body.is_empty() => format!("{greeting}\n{}", capitalize(body)),
        _ => text.to_owned(),
    }
}

/// The most words a chat message can have and still read as a quick reply.
const SHORT_CHAT_WORDS: usize = 12;

/// Chat apps only: a short one-sentence message has no final period.
pub fn drop_chat_period(text: &str) -> String {
    let short = text.split_whitespace().count() <= SHORT_CHAT_WORDS && !text.contains('\n');
    match without_final_period(text) {
        Some(rest) if short && !has_sentence_break(rest) => rest.to_owned(),
        _ => text.to_owned(),
    }
}

fn has_sentence_break(text: &str) -> bool {
    [". ", "! ", "? ", "… "].iter().any(|b| text.contains(b))
}

/// Matches a dictation's first letter to the text before the cursor: rule cleanup capitalized it
/// as a sentence start, which is wrong mid-sentence. A proper noun looks the same, so it only
/// survives if the dictionary lists it (replacements run after this pass).
pub fn match_casing(text: &str, before: Option<&str>) -> String {
    match before {
        Some(before) if !at_sentence_start(before) => lowercase_first_word(text),
        _ => text.to_owned(),
    }
}

/// Whether text typed after `before` starts a sentence. A line with no words yet (an empty line,
/// a list marker) counts as one.
fn at_sentence_start(before: &str) -> bool {
    let line = before.rsplit('\n').next().unwrap_or(before);
    let closed = line
        .trim_end()
        .trim_end_matches(['"', '\'', ')', ']', '”', '’', '»']);
    !line.contains(char::is_alphanumeric) || closed.ends_with(['.', '!', '?', '…'])
}

/// Lowercases a capital that only marks a sentence start: "I" and words with inner capitals
/// ("NASA", "GitHub") are written that way everywhere.
fn lowercase_first_word(text: &str) -> String {
    let Some(start) = text.find(char::is_alphanumeric) else {
        return text.to_owned();
    };
    let word = text[start..].split(|c: char| !c.is_alphanumeric()).next();
    let mut chars = word.unwrap_or_default().chars();
    let Some(first) = chars.next() else {
        return text.to_owned();
    };
    if word == Some("I") || chars.any(char::is_uppercase) {
        return text.to_owned();
    }
    let lowered: String = first.to_lowercase().collect();
    format!(
        "{}{lowered}{}",
        &text[..start],
        &text[start + first.len_utf8()..]
    )
}

/// Characters a word attaches to with no space, including the Italian elision ("l'amico").
const OPENERS: &[char] = &['(', '[', '{', '"', '\'', '’', '‘', '“', '«', '-', '/'];

/// Fits the text between its neighbours: the leading space the text before the cursor calls for,
/// and no punctuation doubled on either side. Unknown neighbours (`None`) change nothing.
pub fn match_spacing(text: &str, before: Option<&str>, after: Option<&str>) -> String {
    let mut body = text;
    let mut leading_space = false;
    if let Some(previous) = before.and_then(|b| b.chars().next_back()) {
        if MARKS.contains(&previous) {
            body = body.trim_start_matches(MARKS);
        }
        if previous.is_whitespace() {
            body = body.trim_start();
        }
        leading_space = !previous.is_whitespace()
            && !OPENERS.contains(&previous)
            && body.starts_with(|c: char| c.is_alphanumeric() || OPENERS.contains(&c));
    }
    if after.is_some_and(|a| a.starts_with(MARKS)) {
        body = body.trim_end_matches(MARKS);
    }
    if leading_space {
        format!(" {body}")
    } else {
        body.to_owned()
    }
}

/// Punctuation that can end a word, and so double up where two texts meet.
const MARKS: [char; 7] = ['.', ',', ';', ':', '!', '?', '…'];

/// Spoken commands that press Enter after the text. Lowercase.
const PRESS_ENTER: &[&str] = &["press enter", "premi invio", "premi enter"];

/// If the dictation ends with the press-enter command, the text without it. Sentence-final
/// punctuation moves onto what remains. Empty when the command was all that was said.
pub fn strip_press_enter(text: &str) -> Option<String> {
    let body = text.trim_end_matches(|c: char| MARKS.contains(&c) || c.is_whitespace());
    let command = PRESS_ENTER
        .iter()
        .find_map(|command| phrase_start_at_end(body, command))?;
    let kept = body[..command]
        .trim_end_matches(|c: char| c.is_whitespace() || [',', ';', ':'].contains(&c));
    if kept.is_empty() || kept.ends_with(['.', '!', '?', '…']) {
        return Some(kept.to_owned());
    }
    let end = text[body.len()..].trim().trim_matches([',', ';', ':']);
    Some(format!("{kept}{end}"))
}

/// Where `phrase` starts, if it is what `text` ends with.
fn phrase_start_at_end(text: &str, phrase: &str) -> Option<usize> {
    let starts = text.char_indices().map(|(i, _)| i);
    starts
        .filter(|&i| at_word_start(text, i))
        .find(|&i| phrase_len(&text[i..], phrase) == Some(text.len() - i))
}

/// The snippet this dictation consists of, if it says nothing but a trigger.
pub fn whole_snippet<'a>(text: &str, snippets: &'a [Snippet]) -> Option<&'a str> {
    let body = text
        .trim()
        .trim_end_matches(|c: char| MARKS.contains(&c) || c.is_whitespace());
    snippets
        .iter()
        .find(|s| phrase_len(body, &s.trigger) == Some(body.len()))
        .map(|s| s.text.as_str())
}

/// Expands every trigger inside the text. Expansions are the user's stored text, so they are
/// written verbatim and never expanded again.
pub fn expand_snippets(text: &str, snippets: &[Snippet]) -> String {
    let rules: Vec<(&str, &str)> = snippets
        .iter()
        .map(|s| (s.trigger.as_str(), s.text.as_str()))
        .collect();
    substitute(text, &rules, |_, text| text.to_owned())
}

/// Applies the dictionary. A right side with capitals of its own ("GitHub") is written exactly;
/// an all-lowercase one takes the capital of a sentence start ("Teh" → "The").
pub fn apply_replacements(text: &str, replacements: &[Replacement]) -> String {
    let rules: Vec<(&str, &str)> = replacements
        .iter()
        .map(|r| (r.wrong.as_str(), r.right.as_str()))
        .collect();
    substitute(text, &rules, |matched, right| {
        let capitalized = matched.starts_with(char::is_uppercase);
        if capitalized && !right.contains(char::is_uppercase) {
            capitalize(right)
        } else {
            right.to_owned()
        }
    })
}

/// Rewrites whole-word occurrences of each `(from, to)` in one left-to-right scan: the longest
/// `from` wins at each position and written text is not scanned again. `write` renders a hit.
fn substitute(text: &str, rules: &[(&str, &str)], write: impl Fn(&str, &str) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while let Some(next) = text[i..].chars().next() {
        let hit = rules
            .iter()
            .filter(|_| at_word_start(text, i))
            .filter_map(|&(from, to)| Some((phrase_len(&text[i..], from)?, to)))
            .max_by_key(|&(len, _)| len);
        let Some((len, to)) = hit else {
            out.push(next);
            i += next.len_utf8();
            continue;
        };
        out.push_str(&write(&text[i..i + len], to));
        i += len;
        if to.ends_with(MARKS) {
            i = text.len() - text[i..].trim_start_matches(MARKS).len();
        }
    }
    out
}

fn at_word_start(text: &str, i: usize) -> bool {
    !text[..i].ends_with(char::is_alphanumeric)
}

/// The byte length of `phrase` at the start of `text`, ignoring case and how the words are
/// spaced, if it ends on a word boundary.
fn phrase_len(text: &str, phrase: &str) -> Option<usize> {
    let mut rest = text;
    for (i, word) in phrase.split_whitespace().enumerate() {
        if i > 0 {
            let spaced = rest.trim_start();
            if spaced.len() == rest.len() {
                return None;
            }
            rest = spaced;
        }
        rest = strip_ignoring_case(rest, word)?;
    }
    let len = text.len() - rest.len();
    (len > 0 && !rest.starts_with(char::is_alphanumeric)).then_some(len)
}

fn strip_ignoring_case<'a>(text: &'a str, word: &str) -> Option<&'a str> {
    let mut chars = text.chars();
    for expected in word.chars() {
        if !chars.next()?.to_lowercase().eq(expected.to_lowercase()) {
            return None;
        }
    }
    Some(chars.as_str())
}
