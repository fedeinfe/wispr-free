//! Rule cleanup (spec §3): the deterministic pass between ASR and LLM cleanup. It removes
//! fillers, collapses stutters, applies spoken edit commands and simple slot self-corrections,
//! then sentence-cases the result so the text is usable when LLM cleanup is off.

/// Cleans an ASR transcript. `language` is the transcript's BCP-47-ish code ("en", "it-IT");
/// `None` or an unknown language applies every language's word lists.
pub fn rule_cleanup(text: &str, language: Option<&str>) -> String {
    let lexicons = lexicons_for(language);
    let mut words = parse_words(text);
    // Each pass only deletes words and one can expose work for another (collapsing
    // "diciamo diciamo," sets off a filler), so repeat until nothing changes.
    loop {
        let before = words.len();
        remove_fillers(&mut words, &lexicons);
        collapse_repeats(&mut words, &lexicons);
        apply_edit_commands(&mut words, &lexicons);
        apply_restarts(&mut words, &lexicons);
        apply_self_corrections(&mut words, &lexicons);
        if words.len() == before {
            break;
        }
    }
    close_pauses(&mut words);
    sentence_case(&mut words);
    render(&words)
}

/// The words of an ASR transcript. Whisper glues its pauses to the words around them, as in
/// "ehm...scarica", so they're cut free before anything compares a word.
fn parse_words(text: &str) -> Vec<Word> {
    let text = text.replace('…', PAUSE).replace(PAUSE, "... ");
    let mut words: Vec<Word> = Vec::new();
    for word in text.split_whitespace().map(Word::parse) {
        if word.key.is_empty() && word.prefix.chars().all(|c| c == '.') {
            if let Some(last) = words.last_mut()
                && !last.suffix.contains(PAUSE)
            {
                last.suffix.push_str(PAUSE);
            }
        } else if word.key.is_empty()
            && let Some(last) = words.last_mut().filter(|last| last.suffix.ends_with(PAUSE))
        {
            last.suffix.push_str(&word.prefix);
        } else {
            words.push(word);
        }
    }
    words
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
pub fn new_words(text: &str, reply: &str) -> (usize, usize) {
    let spoken: Vec<Word> = text.split_whitespace().map(Word::parse).collect();
    let words = reply.split_whitespace().map(Word::parse);
    let words: Vec<Word> = words.filter(|word| !word.key.is_empty()).collect();
    let new = words
        .iter()
        .filter(|word| !spoken.iter().any(|said| said.key == word.key))
        .count();
    (new, words.len())
}

/// Words that turn into digits and symbols, so a reply loses nothing by dropping them.
const SPOKEN_FORMAT: &[&str] = &[
    "punto",
    "virgola",
    "chiocciola",
    "barra",
    "percento",
    "dot",
    "point",
    "slash",
    "percent",
];

/// The most words of `text` gone from `reply` in a row. Numbers, short words and spoken formatting
/// don't count, and a word survives inside another, as "rossi" does in "mario.rossi@gmail.com".
pub fn longest_cut(text: &str, reply: &str) -> usize {
    let reply = reply.to_lowercase();
    let (mut run, mut longest) = (0, 0);
    for word in text.split_whitespace().map(Word::parse) {
        let key = word.key.as_str();
        if key.chars().count() < 3 || is_number(key) || SPOKEN_FORMAT.contains(&key) {
            continue;
        }
        run = if reply.contains(key) { 0 } else { run + 1 };
        longest = longest.max(run);
    }
    longest
}

/// The first word, lowercase and without its punctuation.
pub(crate) fn first_word(text: &str) -> Option<String> {
    text.split_whitespace().next().map(|w| Word::parse(w).key)
}

/// Per-language word lists. Entries are lowercase.
struct Lexicon {
    language: &'static str,
    /// Revisions so clear that they count without a comma before them.
    plain_revisions: &'static [&'static str],
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
        plain_revisions: &["no wait", "no sorry", "actually no"],
        fillers: &["um", "uh", "uhm", "er", "erm", "hm", "mh", "mhm"],
        filler_phrases: &["you know", "ah", "oh"],
        edit_commands: &["scratch that"],
        edit_commands_at_pause: &["delete that", "never mind"],
        correction_markers: &[
            "no",
            "no wait",
            "wait",
            "wait no",
            "sorry",
            "no sorry",
            "sorry no",
            "oh no",
            "i mean",
            "i meant",
            "actually",
            "actually no",
            "no actually",
            "on second thought",
        ],
        doubles: &["had", "bye"],
    },
    Lexicon {
        language: "it",
        plain_revisions: &["cioè no", "anzi no", "no aspetta", "no scusa"],
        fillers: &["ehm", "emh", "eh", "um", "uh", "uhm", "hm", "mh", "mhm"],
        filler_phrases: &["diciamo", "tipo", "ah", "oh"],
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
            "aspetta no",
            "anzi",
            "anzi no",
            "no anzi",
            "cioè",
            "cioè no",
            "scusa",
            "no scusa",
            "scusa no",
            "ah no",
            "ma no",
            "volevo dire",
            "intendevo",
            "ripensandoci",
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

/// Hesitations, whatever language they interrupt. See [`is_drawn_out`].
const HESITATIONS: &[&str] = &[
    "ah", "eh", "ehm", "emh", "hm", "m", "mh", "mhm", "oh", "uh", "uhm", "um",
];

/// A hesitation held for longer, as in "eeehm", "mmmh" or "ohhh". No word squeezes down to one.
fn is_drawn_out(key: &str) -> bool {
    let mut squeezed: Vec<char> = key.chars().collect();
    squeezed.dedup();
    let squeezed = String::from_iter(squeezed);
    squeezed.len() < key.len() && HESITATIONS.contains(&squeezed.as_str())
}

/// Whether a revision of several words starts at `i`, as "oh, no" does. Its first word isn't a
/// filler, even set off.
fn revision_at(words: &[Word], i: usize, lexicons: &[&Lexicon]) -> bool {
    lexicons
        .iter()
        .flat_map(|l| l.correction_markers)
        .any(|marker| marker.contains(' ') && keys_at(words, i, marker).is_some())
}

fn remove_fillers(words: &mut Vec<Word>, lexicons: &[&Lexicon]) {
    let mut i = 0;
    while i < words.len() {
        let key = words[i].key.as_str();
        let filler =
            (lexicons.iter().any(|l| l.fillers.contains(&key)) || is_drawn_out(key)).then_some(1);
        let span = filler.or_else(|| {
            lexicons
                .iter()
                .flat_map(|l| l.filler_phrases)
                .filter_map(|p| phrase_at(words, i, p))
                .find(|&n| set_off(words, i, n) && !revision_at(words, i, lexicons))
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
    // A pause belongs to the hesitation it came with.
    if previous.ends_sentence() || removed.contains(PAUSE) {
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

/// The longest revision marker of any kind starting at `i`, in words.
fn revision_marker(words: &[Word], i: usize, lexicons: &[&Lexicon]) -> Option<usize> {
    lexicons
        .iter()
        .flat_map(|l| l.correction_markers)
        .filter_map(|marker| keys_at(words, i, marker))
        .max()
}

/// After the verb these point back at a name the revision would take away: "tell Anna this, no,
/// tell her that" still means Anna, which takes understanding.
const POINTING_BACK: &[&str] = &["her", "him", "them", "lei", "lui", "loro"];

/// How many more words a revision may take back than it says itself. Past that, as in "send Mark
/// the file, no, to Luke", it would take away something the speaker kept.
const RESTART_SLACK: usize = 1;

/// Resolves "A, marker, B" when B restarts from a word of A: everything from that word on is taken
/// back, so "fai questa cosa, cioè no, fai quest'altra" leaves "fai quest'altra". Unlike the slot
/// swap this keeps what follows B, and it works without commas when the marker is unmistakable.
/// Revisions with no word in common need to be understood, so they stay for LLM cleanup.
fn apply_restarts(words: &mut Vec<Word>, lexicons: &[&Lexicon]) {
    let mut i = 1;
    while i < words.len() {
        if let Some(range) = restart_at(words, i, lexicons) {
            i = range.start.max(1);
            words.drain(range);
        } else {
            i += 1;
        }
    }
}

/// The words a revision at `i` takes back, marker included.
fn restart_at(words: &[Word], i: usize, lexicons: &[&Lexicon]) -> Option<std::ops::Range<usize>> {
    let plain = lexicons
        .iter()
        .flat_map(|l| l.plain_revisions)
        .any(|marker| keys_at(words, i, marker).is_some());
    if !plain && !words[i - 1].suffix.contains(',') {
        return None;
    }
    let key = |at: usize| words.get(at).map(|word| word.key.as_str());
    // "tutti i video, cioè tutti no, solo quelli": the word repeated says where it starts.
    let repeated = key(i) == Some("cioè") && key(i + 2) == Some("no");
    let mut first = if repeated { i + 3 } else { i };
    while let Some(marker) = revision_marker(words, first, lexicons) {
        first += marker;
    }
    let explained = first == i + 1 && key(i) == Some("cioè");
    if first == i || explained || words[first - 1].suffix.contains('?') {
        return None;
    }
    let anchor = if repeated { key(i + 1) } else { key(first) }?;
    if key(first + 1).is_some_and(|next| POINTING_BACK.contains(&next)) {
        return None;
    }

    let clause = (0..i - 1)
        .rev()
        .find(|&at| words[at].ends_clause())
        .map_or(0, |at| at + 1);
    let from = (clause..i).rev().find(|&at| words[at].key == anchor)?;
    let last = (first..words.len()).find(|&at| words[at].ends_clause());
    let said = last.unwrap_or(words.len() - 1) + 1 - first;
    (i - from <= said + RESTART_SLACK).then_some(from..first)
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
        if let Some(m) = marker.filter(|&m| !words[i + m - 1].suffix.contains('?'))
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
    // "manda la mail a Marco, anzi no, mandala a Luca" starts over, whatever else lines up.
    let restarts = words[sentence_start..i]
        .iter()
        .any(|word| word.key.len() > 3 && words[first].key.starts_with(&word.key));
    if restarts {
        return None;
    }
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

/// How Whisper writes a hesitation.
const PAUSE: &str = "...";

/// A phrase can't end on these, so a pause after one is the speaker looking for the next word.
const OPEN_WORDS: &[&str] = &[
    "di", "a", "da", "in", "con", "su", "per", "tra", "fra", "il", "lo", "la", "l", "i", "gli",
    "le", "un", "una", "uno", "the", "an", "to", "of", "with", "for",
];
/// [`OPEN_WORDS`] that join clauses, so a capital after their pause is Whisper's and not a name.
const CONNECTIVES: &[&str] = &["e", "ed", "o", "ma", "che", "and", "or", "but", "that"];

/// Writes each pause as the punctuation it stands for: nothing inside a phrase, a sentence end
/// where Whisper went on with a capital, and otherwise a comma.
fn close_pauses(words: &mut [Word]) {
    for i in 0..words.len() {
        if !words[i].suffix.contains(PAUSE) {
            continue;
        }
        let key = words[i].key.as_str();
        let (open, connective) = (OPEN_WORDS.contains(&key), CONNECTIVES.contains(&key));
        let closing = match words.get_mut(i + 1) {
            None => ".",
            Some(next) if connective => {
                next.core = lowercase_start(&next.core);
                ""
            }
            Some(_) if open => "",
            Some(next) if next.core.starts_with(char::is_uppercase) => ".",
            Some(_) => ",",
        };
        let rest = words[i].suffix.replace(PAUSE, "");
        words[i].suffix = if rest.is_empty() {
            closing.to_owned()
        } else {
            rest
        };
    }
}

/// Lowers a first letter, leaving "I" and acronyms as they are.
fn lowercase_start(word: &str) -> String {
    let mut chars = word.chars();
    match (chars.next(), chars.next()) {
        (Some(first), Some(second)) if !second.is_uppercase() => first
            .to_lowercase()
            .chain(word[first.len_utf8()..].chars())
            .collect(),
        _ => word.to_owned(),
    }
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
