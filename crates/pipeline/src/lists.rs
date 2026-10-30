//! Spoken lists (spec §3): "these three things: a, b, c" and "first a, second b, third c" written
//! as lists. Fixed rules rather than the LLM, so the same words always give the same layout, on
//! every tier and with cleanup off. They only fire on a clear announcement or count: a list that
//! stays a sentence costs less than a sentence that gets cut into a list.

use crate::rules::capitalize;

/// The longest bullet in words. Longer than this, the commas aren't separating items.
const MAX_BULLET_WORDS: usize = 10;

/// `(value, spoken forms)` of the counting words.
const ORDINALS: &[(usize, &[&str])] = &[
    (1, &["primo", "prima", "first", "firstly"]),
    (2, &["secondo", "seconda", "second", "secondly"]),
    (3, &["terzo", "terza", "third", "thirdly"]),
    (4, &["quarto", "quarta", "fourth"]),
    (5, &["quinto", "quinta", "fifth"]),
    (6, &["sesto", "sesta", "sixth"]),
    (7, &["settimo", "settima", "seventh"]),
    (8, &["ottavo", "ottava", "eighth"]),
    (9, &["nono", "nona", "ninth"]),
    (10, &["decimo", "decima", "tenth"]),
];

const CARDINALS: &[(usize, &[&str])] = &[
    (1, &["uno", "one"]),
    (2, &["due", "two"]),
    (3, &["tre", "three"]),
    (4, &["quattro", "four"]),
    (5, &["cinque", "five"]),
    (6, &["sei", "six"]),
    (7, &["sette", "seven"]),
    (8, &["otto", "eight"]),
    (9, &["nove", "nine"]),
    (10, &["dieci", "ten"]),
];

/// Before an ordinal these make it an adjective: "il secondo tempo", "in first place".
const NOT_BEFORE_A_MARKER: &[&str] = &[
    "il", "lo", "la", "l", "i", "gli", "le", "un", "una", "al", "allo", "alla", "all", "ai",
    "agli", "alle", "del", "dello", "della", "dell", "dei", "degli", "delle", "dal", "dalla",
    "dai", "dalle", "nel", "nello", "nella", "nell", "nei", "nelle", "sul", "sulla", "sui",
    "sulle", "di", "a", "da", "in", "con", "su", "per", "tra", "fra", "mio", "mia", "tuo", "tua",
    "suo", "sua", "nostro", "nostra", "questo", "questa", "quel", "quella", "the", "an", "my",
    "your", "his", "her", "our", "their", "this", "that", "at", "on", "of", "for", "one", "came",
];

/// After "secondo" these make it "according to": "secondo me", "secondo quanto dici".
const NOT_AFTER_SECONDO: &[&str] = &[
    "me", "te", "lui", "lei", "noi", "voi", "loro", "quanto", "cui", "come",
];

/// Before a counting word these introduce the item when a break or "è che" says so: "il primo è
/// che…". Otherwise they make it an adjective.
const ARTICLES: &[&str] = &["il", "la", "lo", "l", "the"];

/// Spoken right after a counting word, as in "prima cosa", without being part of the item.
const COUNTED_NOUNS: &[&str] = &[
    "cosa", "punto", "problema", "motivo", "bug", "thing", "point", "item", "problem", "reason",
    "issue", "one",
];

/// What a list can be a list of, after "queste" or "these". Anything else, as in "questi due
/// ragazzi", is too likely to be an ordinary sentence.
const LIST_NOUNS: &[&str] = &[
    "cose",
    "punti",
    "elementi",
    "voci",
    "attività",
    "task",
    "passaggi",
    "step",
    "bug",
    "problemi",
    "motivi",
    "idee",
    "domande",
    "things",
    "items",
    "points",
    "steps",
    "tasks",
    "bugs",
    "issues",
    "problems",
    "reasons",
    "ideas",
    "questions",
];

/// In the lead-in, these and a count announce a list, which lets two counted items be enough.
const ANNOUNCING: &[&str] = &["questi", "queste", "these", "seguenti", "following"];

const JOINERS: &[&str] = &[
    "e",
    "ed",
    "and",
    "poi",
    "quindi",
    "dopodiché",
    "then",
    "infine",
    "finally",
];

/// The transcript with its list written out, or `None` when it isn't one.
pub fn format_lists(text: &str) -> Option<String> {
    if text.contains('\n') {
        return None;
    }
    let words = words(text);
    numbered(text, &words).or_else(|| bulleted(text, &words))
}

struct Word {
    text: String,
    start: usize,
    end: usize,
}

fn words(text: &str) -> Vec<Word> {
    let mut words = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices().chain([(text.len(), ' ')]) {
        match (c.is_alphanumeric(), start) {
            (true, None) => start = Some(i),
            (false, Some(from)) => {
                words.push(Word {
                    text: text[from..i].to_lowercase(),
                    start: from,
                    end: i,
                });
                start = None;
            }
            _ => {}
        }
    }
    words
}

fn value_in(table: &[(usize, &[&str])], word: &str) -> Option<usize> {
    let hit = table.iter().find(|(_, forms)| forms.contains(&word));
    hit.map(|&(value, _)| value)
}

fn count_of(word: &str) -> Option<usize> {
    value_in(CARDINALS, word).or(word.parse().ok())
}

fn is_separator(c: char) -> bool {
    c.is_whitespace() || ",;:.".contains(c)
}

/// Whether punctuation, or nothing at all, comes before this byte.
fn after_a_break(text: &str, at: usize) -> bool {
    let before = text[..at].trim_end();
    before.is_empty() || before.ends_with([',', ';', ':', '.', '!', '?'])
}

fn then_punctuation(text: &str, at: usize) -> bool {
    text[at..].starts_with([',', ':', '.'])
}

struct Marker {
    value: usize,
    start: usize,
    end: usize,
    /// An article and no "è che", as in "il secondo no": it needs the lead-in to announce a list.
    weak: bool,
    /// An article after "e" or "poi" rather than a break, as in "e poi la terza".
    joined: bool,
}

/// Where the words that lead into an item end, after the ordinal at `words[i]`, and whether they
/// close with "è che" or "is that".
fn after_ordinal(words: &[Word], i: usize) -> (usize, bool) {
    let is = |at: usize, wanted: &[&str]| {
        let word = words.get(at);
        word.is_some_and(|word| wanted.contains(&word.text.as_str()))
    };
    let mut last = i;
    if is(last + 1, COUNTED_NOUNS) {
        last += 1;
    }
    if !is(last + 1, &["è", "is"]) {
        return (words[last].end, false);
    }
    last += 1;
    let closed = is(last + 1, &["che", "that"]);
    if closed {
        last += 1;
    }
    (words[last].end, closed)
}

/// The counting words that start at `words[i]`, if that's what they are.
fn marker(text: &str, words: &[Word], i: usize) -> Option<Marker> {
    let word = &words[i];
    let (next, after_next) = (words.get(i + 1), words.get(i + 2));
    let next_is = |wanted: &str| next.is_some_and(|word| word.text == wanted);
    let previous = i.checked_sub(1).map(|p| &words[p]);

    if let Some(value) = value_in(ORDINALS, &word.text) {
        if next_is("luogo") && previous.is_some_and(|word| word.text == "in") {
            let start = previous?.start;
            return next.map(|next| Marker {
                value,
                start,
                end: next.end,
                weak: false,
                joined: false,
            });
        }
        let opener = (word.text == "prima" && next_is("di"))
            .then(|| after_next.filter(|word| word.text == "tutto"))
            .flatten()
            .or((word.text == "first" && next_is("of"))
                .then(|| after_next.filter(|word| word.text == "all"))
                .flatten());
        if let Some(opener) = opener {
            return Some(Marker {
                value,
                start: word.start,
                end: opener.end,
                weak: false,
                joined: false,
            });
        }
        let (end, closed) = after_ordinal(words, i);
        if let Some(article) = previous.filter(|word| ARTICLES.contains(&word.text.as_str())) {
            let broken = closed || after_a_break(text, article.start);
            let before_article = i.checked_sub(2).map(|p| words[p].text.as_str());
            let joined = !broken && before_article.is_some_and(|word| JOINERS.contains(&word));
            if broken || joined {
                return Some(Marker {
                    value,
                    start: article.start,
                    end,
                    weak: !closed,
                    joined,
                });
            }
        }
        let adjective = !after_a_break(text, word.start)
            && previous.is_some_and(|word| NOT_BEFORE_A_MARKER.contains(&word.text.as_str()));
        let linked = match word.text.as_str() {
            "secondo" => next.is_some_and(|word| NOT_AFTER_SECONDO.contains(&word.text.as_str())),
            "prima" => next_is("di") || next_is("che") || next_is("della") || next_is("del"),
            _ => false,
        };
        if adjective || linked {
            return None;
        }
        return Some(Marker {
            value,
            start: word.start,
            end,
            weak: false,
            joined: false,
        });
    }

    if ["punto", "numero", "point", "number"].contains(&word.text.as_str()) {
        let next = next?;
        let value = count_of(&next.text)?;
        return Some(Marker {
            value,
            start: word.start,
            end: next.end,
            weak: false,
            joined: false,
        });
    }

    if let Ok(value) = word.text.parse() {
        let spaced = text[word.end..]
            .chars()
            .nth(1)
            .is_none_or(char::is_whitespace);
        return (text[word.end..].starts_with(['.', ')', ':']) && spaced).then_some(Marker {
            value,
            start: word.start,
            end: word.end + 1,
            weak: false,
            joined: false,
        });
    }

    let value = value_in(CARDINALS, &word.text)?;
    (after_a_break(text, word.start) && then_punctuation(text, word.end)).then_some(Marker {
        value,
        start: word.start,
        end: word.end,
        weak: false,
        joined: false,
    })
}

/// Cuts `text` at its first sentence end, where what follows starts with a capital.
fn first_sentence(text: &str) -> (&str, &str) {
    for (i, c) in text.char_indices() {
        if !".!?".contains(c) {
            continue;
        }
        let rest = &text[i + c.len_utf8()..];
        let next = rest.trim_start().chars().next();
        if rest.starts_with(char::is_whitespace) && next.is_some_and(char::is_uppercase) {
            return (&text[..i + c.len_utf8()], rest.trim_start());
        }
    }
    (text, "")
}

/// An item without the punctuation and the "and" around it. `None` when nothing is left.
fn item(text: &str) -> Option<&str> {
    let mut item = text.trim_matches(is_separator);
    while let Some((rest, last)) = item.rsplit_once(' ')
        && JOINERS.contains(&last.to_lowercase().as_str())
    {
        item = rest.trim_end_matches(is_separator);
    }
    (!item.is_empty()).then_some(item)
}

fn lead_in(text: &str) -> String {
    let lead = text.trim_matches(is_separator);
    if lead.is_empty() || lead.ends_with(['?', '!']) {
        lead.to_owned()
    } else {
        format!("{lead}:")
    }
}

fn layout(lead: &str, lines: &[String], tail: &str) -> String {
    let mut written = lead.to_owned();
    for line in lines {
        if !written.is_empty() {
            written.push('\n');
        }
        written.push_str(line);
    }
    if !tail.is_empty() {
        written.push_str("\n\n");
        written.push_str(tail);
    }
    written
}

fn numbered(text: &str, words: &[Word]) -> Option<String> {
    let mut found: Vec<Marker> = Vec::new();
    for i in 0..words.len() {
        if let Some(marker) = marker(text, words, i) {
            let overlaps = found.last().is_some_and(|last| marker.start < last.end);
            if marker.value == found.len() + 1 && !overlaps {
                found.push(marker);
            }
        }
    }
    let lead = &text[..found.first()?.start];
    let announced = || {
        let announcing = |word: &Word| {
            ANNOUNCING.contains(&word.text.as_str()) || count_of(&word.text).is_some_and(|n| n > 1)
        };
        words
            .iter()
            .take_while(|word| word.start < lead.len())
            .any(announcing)
    };
    let weak = found.iter().any(|marker| marker.weak);
    if found.len() < 2 || ((found.len() == 2 || weak) && !announced()) {
        return None;
    }
    // "Ho due figli, la prima… e la seconda…" is a sentence, even with its count.
    let joined = found.iter().any(|marker| marker.joined);
    if found.len() == 2 && joined && announcement(text, words).is_none() {
        return None;
    }

    let mut lines = Vec::with_capacity(found.len());
    for (n, pair) in found.windows(2).enumerate() {
        let spoken = item(&text[pair[0].end..pair[1].start])?;
        lines.push(format!("{}. {spoken}", n + 1));
    }
    let (last, tail) = first_sentence(&text[found.last()?.end..]);
    lines.push(format!("{}. {}", found.len(), item(last)?));
    Some(layout(&lead_in(lead), &lines, tail))
}

/// Where the announcement ends, and how many items it promised.
fn announcement(text: &str, words: &[Word]) -> Option<(usize, Option<usize>)> {
    for (i, word) in words.iter().enumerate() {
        let noun_at = |at: usize| {
            words
                .get(at)
                .filter(|word| LIST_NOUNS.contains(&word.text.as_str()))
        };
        if ["queste", "questi", "these"].contains(&word.text.as_str()) {
            let count = words.get(i + 1).and_then(|word| count_of(&word.text));
            let noun = noun_at(if count.is_some() { i + 2 } else { i + 1 });
            if let Some(noun) = noun
                && (count.is_some() || text[noun.end..].starts_with(':'))
            {
                return Some((noun.end, count));
            }
        }
        if ["seguenti", "following"].contains(&word.text.as_str())
            && let Some(noun) = words.get(i + 1)
        {
            return Some((noun.end, None));
        }
        if let Some(count) = count_of(&word.text)
            && let Some(noun) = words.get(i + 1)
            && text[noun.end..].starts_with(':')
        {
            return Some((noun.end, Some(count)));
        }
    }
    None
}

/// Splits the last item at its "and", which is how a spoken list usually ends.
fn split_last(parts: &mut Vec<&str>) -> bool {
    let Some(last) = parts.last().copied() else {
        return false;
    };
    let found = [" e ", " ed ", " and "]
        .iter()
        .find_map(|joiner| last.rsplit_once(joiner));
    let Some((before, after)) = found else {
        return false;
    };
    parts.pop();
    parts.extend([before, after]);
    true
}

/// The items after the announcement and the text left after them. Whisper may end a sentence
/// after each item, so a promised count can reach past the first sentence.
fn spoken_items(text: &str, count: Option<usize>) -> Option<(Vec<&str>, &str)> {
    let mut sentences = Vec::new();
    let mut rest = text;
    loop {
        let (sentence, tail) = first_sentence(rest);
        sentences.push(sentence);
        rest = tail;
        let mut parts: Vec<&str> = sentences
            .iter()
            .flat_map(|sentence| sentence.split([',', ';']))
            .collect();
        let Some(count) = count else {
            split_last(&mut parts);
            return (parts.len() >= 2).then_some((parts, rest));
        };
        while parts.len() < count && split_last(&mut parts) {}
        if parts.len() == count {
            return Some((parts, rest));
        }
        if parts.len() > count || rest.is_empty() {
            return None;
        }
    }
}

fn bulleted(text: &str, words: &[Word]) -> Option<String> {
    let (end, count) = announcement(text, words)?;
    let mut lead = &text[..end];
    let mut body = text[end..].trim_start_matches(is_separator);
    // "queste cose da fare: a, b": the announcement runs up to its colon.
    if let Some((more, rest)) = first_sentence(body).0.split_once(':')
        && !more.contains(',')
    {
        lead = &text[..text.len() - body.len() + more.len()];
        body = rest;
    }
    let (parts, tail) = spoken_items(body, count)?;

    let mut lines = Vec::with_capacity(parts.len());
    for part in parts {
        let spoken = item(part)?;
        if spoken.split_whitespace().count() > MAX_BULLET_WORDS {
            return None;
        }
        lines.push(format!("- {}", capitalize(without_going(spoken))));
    }
    Some(layout(&lead_in(lead), &lines, tail))
}

/// "andare a fare la spesa" reads as "fare la spesa" in a to-do list. "andare a Roma" stays.
fn without_going(item: &str) -> &str {
    let lower = item.to_lowercase();
    let Some(rest) = lower.strip_prefix("andare a ") else {
        return item;
    };
    let verb = rest.split_whitespace().next().unwrap_or_default();
    if ["are", "ere", "ire"]
        .iter()
        .any(|ending| verb.ends_with(ending))
    {
        &item["andare a ".len()..]
    } else {
        item
    }
}
