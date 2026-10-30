//! Rule cleanup (spec §3): the deterministic pass between ASR and LLM cleanup. It removes
//! fillers, collapses stutters, applies spoken edit commands and simple slot self-corrections,
//! then sentence-cases the result so the text is usable when LLM cleanup is off.

/// Cleans an ASR transcript. `language` is the transcript's BCP-47-ish code ("en", "it-IT");
/// `None` or an unknown language applies every language's word lists.
pub fn rule_cleanup(text: &str, language: Option<&str>) -> String {
    let lexicons = lexicons_for(language);
    let mut words: Vec<Word> = text.split_whitespace().map(Word::parse).collect();
    // Each pass only deletes words and one can expose work for another (collapsing
    // "diciamo diciamo," sets off a filler), so repeat until nothing changes.
    loop {
        let before = words.len();
        remove_fillers(&mut words, &lexicons);
        collapse_repeats(&mut words, &lexicons);
        apply_edit_commands(&mut words, &lexicons);
        apply_self_corrections(&mut words, &lexicons);
        if words.len() == before {
            break;
        }
    }
    sentence_case(&mut words);
    render(&words)
}

/// Whether the text contains a revision marker of any language ("oh no", "anzi"…).
pub(crate) fn has_correction_marker(text: &str) -> bool {
    let words: Vec<Word> = text.split_whitespace().map(Word::parse).collect();
    LEXICONS
        .iter()
        .flat_map(|lexicon| lexicon.correction_markers)
        .any(|marker| (0..words.len()).any(|i| keys_at(&words, i, marker).is_some()))
}

/// Counts the words in `reply` and how many of them the speaker never said, ignoring case and
/// punctuation. A bare list dash is not a word.
pub(crate) fn new_words(text: &str, reply: &str) -> (usize, usize) {
    let spoken: Vec<Word> = text.split_whitespace().map(Word::parse).collect();
    let words = reply.split_whitespace().map(Word::parse);
    let words: Vec<Word> = words.filter(|word| !word.key.is_empty()).collect();
    let new = words
        .iter()
        .filter(|word| !spoken.iter().any(|said| said.key == word.key))
        .count();
    (new, words.len())
}

/// The first word, lowercase and without its punctuation.
pub(crate) fn first_word(text: &str) -> Option<String> {
    text.split_whitespace().next().map(|w| Word::parse(w).key)
}

/// Per-language word lists. Entries are lowercase.
struct Lexicon {
    language: &'static str,
    /// Disfluency tokens that are never content.
    fillers: &'static [&'static str],
    /// Phrases that are fillers only when set off by punctuation ("so, you know, we…").
    filler_phrases: &'static [&'static str],
    /// Spoken commands that delete the clause or sentence before them.
    edit_commands: &'static [&'static str],
    /// Edit commands that also occur as content ("delete that file", "never mind the rain"),
    /// so they only count right before punctuation or the end of the text.
    edit_commands_at_pause: &'static [&'static str],
    /// Revision markers that, after a comma, introduce a sentence-final replacement
    /// ("six, no, seven"). Inner punctuation is ignored ("no, wait").
    correction_markers: &'static [&'static str],
    /// Words whose doubling is grammatical or idiomatic ("had had"), so never a stutter.
    doubles: &'static [&'static str],
}

const LEXICONS: &[Lexicon] = &[
    Lexicon {
        language: "en",
        fillers: &["um", "umm", "uh", "uhh", "er", "erm", "hmm", "mm"],
        filler_phrases: &["you know"],
        edit_commands: &["scratch that"],
        edit_commands_at_pause: &["delete that", "never mind"],
        correction_markers: &[
            "no", "no wait", "wait", "sorry", "no sorry", "oh no", "i mean", "i meant", "actually",
        ],
        doubles: &["had", "bye"],
    },
    Lexicon {
        language: "it",
        fillers: &["ehm", "eh", "ehh", "uhm", "mmm", "mh"],
        filler_phrases: &["diciamo", "tipo"],
        edit_commands: &[],
        edit_commands_at_pause: &[
            "cancella quello",
            "cancella questo",
            "lascia perdere",
            "lascia stare",
        ],
        correction_markers: &[
            "no",
            "no aspetta",
            "aspetta",
            "anzi",
            "cioè",
            "scusa",
            "volevo dire",
        ],
        doubles: &["così", "piano", "ciao", "via"],
    },
];

fn lexicons_for(language: Option<&str>) -> Vec<&'static Lexicon> {
    let primary = language
        .and_then(|l| l.split(['-', '_']).next())
        .map(str::to_lowercase);
    match LEXICONS
        .iter()
        .find(|l| Some(l.language) == primary.as_deref())
    {
        Some(lexicon) => vec![lexicon],
        None => LEXICONS.iter().collect(),
    }
}

/// One whitespace-separated token, split so rules compare words without their punctuation.
#[derive(Debug, Clone)]
struct Word {
    /// Leading punctuation such as an opening quote or parenthesis.
    prefix: String,
    core: String,
    /// Trailing punctuation such as `,` or `.`.
    suffix: String,
    /// `core` lowercased, what every rule compares.
    key: String,
}

impl Word {
    fn parse(token: &str) -> Word {
        let start = token.find(char::is_alphanumeric).unwrap_or(token.len());
        let end = token
            .trim_end_matches(|c: char| !c.is_alphanumeric())
            .len()
            .max(start);
        let core = &token[start..end];
        Word {
            prefix: token[..start].to_owned(),
            core: core.to_owned(),
            suffix: token[end..].to_owned(),
            key: core.to_lowercase(),
        }
    }

    fn ends_clause(&self) -> bool {
        self.suffix.contains([',', ';', ':', '.', '!', '?', '…'])
    }

    fn ends_sentence(&self) -> bool {
        self.suffix.contains(['.', '!', '?', '…'])
    }
}

fn remove_fillers(words: &mut Vec<Word>, lexicons: &[&Lexicon]) {
    let mut i = 0;
    while i < words.len() {
        let filler = lexicons
            .iter()
            .any(|l| l.fillers.contains(&words[i].key.as_str()))
            .then_some(1);
        let span = filler.or_else(|| {
            lexicons
                .iter()
                .flat_map(|l| l.filler_phrases)
                .filter_map(|p| phrase_at(words, i, p))
                .find(|&n| set_off(words, i, n))
        });
        match span {
            Some(n) => remove_span(words, i, n),
            None => i += 1,
        }
    }
}

/// The word count of `phrase` (lowercase, space-separated) if the words at `i` spell it,
/// ignoring punctuation.
fn keys_at(words: &[Word], i: usize, phrase: &str) -> Option<usize> {
    let mut n = 0;
    for part in phrase.split(' ') {
        if words.get(i + n)?.key != part {
            return None;
        }
        n += 1;
    }
    Some(n)
}

/// Like [`keys_at`], but the phrase must be spoken in one go, with no punctuation inside.
fn phrase_at(words: &[Word], i: usize, phrase: &str) -> Option<usize> {
    keys_at(words, i, phrase).filter(|&n| words[i..i + n - 1].iter().all(|w| w.suffix.is_empty()))
}

/// Whether the `n` words at `i` stand apart: punctuation or the text edge on both sides.
fn set_off(words: &[Word], i: usize, n: usize) -> bool {
    (i == 0 || !words[i - 1].suffix.is_empty()) && pause_after(words, i, n)
}

/// Whether the `n` words at `i` are followed by punctuation or the end of the text.
fn pause_after(words: &[Word], i: usize, n: usize) -> bool {
    i + n == words.len() || !words[i + n - 1].suffix.is_empty()
}

/// Removes `n` words at `i`, keeping the punctuation around the gap sensible.
fn remove_span(words: &mut Vec<Word>, i: usize, n: usize) {
    let removed: Vec<Word> = words.drain(i..i + n).collect();
    if i > 0 {
        absorb_punctuation(&mut words[i - 1], &removed[n - 1].suffix);
    }
}

/// Fixes the punctuation of the word before a removed span whose trailing punctuation was
/// `removed`: a sentence end moves onto the previous word, and a span set off by commas
/// (", uh,") takes both commas with it.
fn absorb_punctuation(previous: &mut Word, removed: &str) {
    if previous.ends_sentence() {
        return;
    }
    let removed_end = removed.trim_start_matches(',');
    if removed_end.contains(['.', '!', '?', '…']) {
        previous.suffix = previous.suffix.trim_end_matches(',').to_owned() + removed_end;
    } else if removed.contains(',') {
        previous.suffix = previous.suffix.trim_end_matches(',').to_owned();
    }
}

/// Longest phrase checked for an immediate repeat ("send me the, send me the").
const MAX_REPEAT_WORDS: usize = 4;

/// Drops the first copy of any word or phrase that is immediately repeated within a sentence.
fn collapse_repeats(words: &mut Vec<Word>, lexicons: &[&Lexicon]) {
    let mut i = 0;
    while i < words.len() {
        let repeat = (1..=MAX_REPEAT_WORDS).find(|&n| {
            i + 2 * n <= words.len()
                && !words[i..i + n].iter().any(Word::ends_sentence)
                && (0..n).all(|k| words[i + k].key == words[i + n + k].key)
                && !(n == 1
                    && lexicons
                        .iter()
                        .any(|l| l.doubles.contains(&words[i].key.as_str())))
                && !words[i..i + n].iter().all(|w| is_number(&w.key))
        });
        match repeat {
            Some(n) => {
                words.drain(i..i + n);
            }
            None => i += 1,
        }
    }
}

/// Deletes each edit command together with the clause before it (back to the previous
/// `,` `;` `:` or sentence end, or the start of an unpunctuated text).
fn apply_edit_commands(words: &mut Vec<Word>, lexicons: &[&Lexicon]) {
    let mut i = 0;
    while i < words.len() {
        let command = lexicons
            .iter()
            .flat_map(|l| l.edit_commands)
            .find_map(|c| phrase_at(words, i, c))
            .or_else(|| {
                lexicons
                    .iter()
                    .flat_map(|l| l.edit_commands_at_pause)
                    .filter_map(|c| phrase_at(words, i, c))
                    .find(|&n| pause_after(words, i, n))
            });
        let Some(n) = command else {
            i += 1;
            continue;
        };
        let mut start = i.saturating_sub(1);
        while start > 0 && !words[start - 1].ends_clause() {
            start -= 1;
        }
        let command_end = words[i + n - 1].suffix.clone();
        words.drain(start..i + n);
        // The clause boundary before the gap stays; only a sentence end needs carrying over.
        if start > 0 && command_end.contains(['.', '!', '?', '…']) {
            absorb_punctuation(&mut words[start - 1], &command_end);
        }
        i = start;
    }
}

/// Longest replacement the slot heuristic trusts; longer revisions are left for LLM cleanup.
const MAX_SLOT_WORDS: usize = 3;

/// Resolves "X, <marker>, Y" at the end of a sentence to Y when Y reads as a swap for the
/// same number of words just before the marker ("at six, no, seven" → "at seven").
fn apply_self_corrections(words: &mut Vec<Word>, lexicons: &[&Lexicon]) {
    let mut sentence_start = 0;
    let mut i = 0;
    while i < words.len() {
        let after_comma = i > sentence_start && words[i - 1].suffix.contains(',');
        let marker = lexicons
            .iter()
            .flat_map(|l| l.correction_markers)
            .filter(|_| after_comma)
            .filter_map(|m| keys_at(words, i, m))
            .max();
        if let Some(m) = marker
            && let Some(slot) = slot_swap(words, sentence_start, i, m)
        {
            words.drain(i - slot..i + m);
            i -= slot;
            continue;
        }
        if words[i].ends_sentence() {
            sentence_start = i + 1;
        }
        i += 1;
    }
}

/// For a marker of `m` words at `i`, the number of words before it that the replacement
/// after it swaps out, if the pattern is a clear slot swap.
/// The replacement runs to the next punctuation and must end the sentence (or the text), be
/// at most [`MAX_SLOT_WORDS`] long, leave part of the sentence in place, and, when longer than
/// one word, share a word with what it replaces ("Tuesday at five" / "Wednesday at five").
fn slot_swap(words: &[Word], sentence_start: usize, i: usize, m: usize) -> Option<usize> {
    let first = i + m;
    if first >= words.len() {
        return None;
    }
    let last = (first..words.len())
        .find(|&r| words[r].ends_clause())
        .unwrap_or(words.len() - 1);
    let n = last + 1 - first;
    let ends_sentence = words[last].ends_sentence() || last + 1 == words.len();
    if n > MAX_SLOT_WORDS || i - sentence_start <= n || !ends_sentence {
        return None;
    }
    let replaced = &words[i - n..i];
    let aligned = if n == 1 {
        !is_number(&words[first].key) || is_number(&replaced[0].key)
    } else {
        replaced
            .iter()
            .zip(&words[first..=last])
            .any(|(a, b)| a.key == b.key)
    };
    aligned.then_some(n)
}

/// Numbers people revise out loud. Larger ones usually span several words and aren't swaps.
const NUMBER_WORDS: &[&str] = &[
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "twenty",
    "thirty",
    "forty",
    "fifty",
    "hundred",
    "thousand",
    "uno",
    "due",
    "tre",
    "quattro",
    "cinque",
    "sei",
    "sette",
    "otto",
    "nove",
    "dieci",
    "undici",
    "dodici",
    "venti",
    "trenta",
    "quaranta",
    "cinquanta",
    "cento",
    "mille",
];

fn is_number(key: &str) -> bool {
    !key.is_empty() && (key.chars().all(|c| c.is_ascii_digit()) || NUMBER_WORDS.contains(&key))
}

/// Capitalizes each sentence start and ends the text with a period if it has no final mark.
fn sentence_case(words: &mut [Word]) {
    let mut at_start = true;
    for word in words.iter_mut() {
        if at_start {
            word.core = capitalize(&word.core);
        }
        at_start = word.ends_sentence();
    }
    if let Some(last) = words.last_mut()
        && !last.ends_sentence()
    {
        last.suffix = last.suffix.trim_end_matches([',', ';', ':']).to_owned() + ".";
    }
}

pub(crate) fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn render(words: &[Word]) -> String {
    words
        .iter()
        .map(|w| format!("{}{}{}", w.prefix, w.core, w.suffix))
        .collect::<Vec<_>>()
        .join(" ")
}
