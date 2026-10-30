//! Golden cases for rule cleanup: ASR text in, the text LLM cleanup (or insertion, when the LLM
//! is off) receives out. Expected outputs are hand-written from the spec, not derived from code.

use wf_pipeline::rules::rule_cleanup;

/// (language, ASR text, expected)
const CASES: &[(Option<&str>, &str, &str)] = &[
    // Spec §8: must hold on every tier, including with the LLM off.
    (
        Some("en"),
        "um so I I think we should uh ship it",
        "So I think we should ship it.",
    ),
    // Punctuated ASR: a filler between commas takes its commas with it.
    (
        Some("en"),
        "Um, so, uh, I think we should ship it.",
        "So I think we should ship it.",
    ),
    (Some("en"), "We should, uh, ship it.", "We should ship it."),
    (Some("en"), "Let's ship it, um.", "Let's ship it."),
    (
        Some("en"),
        "Ship it. Uh, then tell Anna.",
        "Ship it. Then tell Anna.",
    ),
    // Stutters and immediate repeats, also across commas and in multi-word phrases.
    (
        Some("en"),
        "the the report is ready",
        "The report is ready.",
    ),
    (Some("en"), "I, I, I think so.", "I think so."),
    (
        Some("en"),
        "I think, I think we're done.",
        "I think we're done.",
    ),
    (
        Some("en"),
        "Can you send me the, send me the file?",
        "Can you send me the file?",
    ),
    // Repeats that are words, not stutters.
    (Some("en"), "She had had enough.", "She had had enough."),
    (Some("en"), "Ship it. It works.", "Ship it. It works."),
    // "you know" is a filler only when set off by punctuation.
    (
        Some("en"),
        "So, you know, we should ship it.",
        "So we should ship it.",
    ),
    (
        Some("en"),
        "You know, I think it works.",
        "I think it works.",
    ),
    (Some("en"), "It works, you know.", "It works."),
    (
        Some("en"),
        "Do you know the answer?",
        "Do you know the answer?",
    ),
    (Some("en"), "You know what I mean.", "You know what I mean."),
    // Edit commands remove the clause or sentence before them.
    (
        Some("en"),
        "Send it to John, scratch that, send it to Mary.",
        "Send it to Mary.",
    ),
    (Some("en"), "Buy milk. Scratch that. Buy eggs.", "Buy eggs."),
    (Some("en"), "Buy milk. Call Anna, delete that.", "Buy milk."),
    (
        Some("en"),
        "Call Bob, tell him, scratch that, email him.",
        "Call Bob, email him.",
    ),
    (Some("en"), "Ok. Let's meet Friday, never mind.", "Ok."),
    (
        Some("en"),
        "send it to john scratch that send it to mary",
        "Send it to mary.",
    ),
    (Some("en"), "Scratch that.", ""),
    // ...but the same words as content stay.
    (
        Some("en"),
        "Please delete that file.",
        "Please delete that file.",
    ),
    (
        Some("en"),
        "Never mind the weather, let's go.",
        "Never mind the weather, let's go.",
    ),
    // Simple slot self-corrections: the replacement ends the sentence and swaps the words
    // just before the marker. Anything less clear-cut is left for LLM cleanup.
    (
        Some("en"),
        "Let's meet at six, no, seven.",
        "Let's meet at seven.",
    ),
    (
        Some("en"),
        "let's meet at six, no, seven",
        "Let's meet at seven.",
    ),
    (
        Some("en"),
        "Send it to John, sorry, Mary.",
        "Send it to Mary.",
    ),
    (
        Some("en"),
        "Meet on Tuesday at five, no wait, Wednesday at five.",
        "Meet on Wednesday at five.",
    ),
    (
        Some("en"),
        "Meet on Tuesday at five, no, wait, Wednesday at five.",
        "Meet on Wednesday at five.",
    ),
    (
        Some("en"),
        "Book it for Monday, I mean, Tuesday. Thanks.",
        "Book it for Tuesday. Thanks.",
    ),
    (
        Some("en"),
        "We need six, I mean, seven chairs.",
        "We need six, I mean, seven chairs.",
    ),
    (
        Some("en"),
        "I like the red one, oh no, I meant the blue one.",
        "I like the red one, oh no, I meant the blue one.",
    ),
    (Some("en"), "I said no, thanks.", "I said no, thanks."),
    // A number can only swap a number: dropping "tickets" here would lose it for good.
    (
        Some("en"),
        "Add two tickets, no, three.",
        "Add two tickets, no, three.",
    ),
    (
        Some("it"),
        "Compra due biglietti, anzi tre.",
        "Compra due biglietti, anzi tre.",
    ),
    (Some("en"), "Add 2, no, 3.", "Add 3."),
    // Repeated digits are content: a phone number is not a stutter.
    (
        Some("en"),
        "My number is five five five one two.",
        "My number is five five five one two.",
    ),
    (
        Some("it"),
        "Il codice è tre tre sette.",
        "Il codice è tre tre sette.",
    ),
    (Some("en"), "Yes, no, maybe.", "Yes, no, maybe."),
    (Some("en"), "Do you want tea, no?", "Do you want tea, no?"),
    (
        Some("en"),
        "let's meet at six no seven",
        "Let's meet at six no seven.",
    ),
    // Italian.
    (
        Some("it"),
        "ehm allora io io penso che eh dovremmo spedirlo",
        "Allora io penso che dovremmo spedirlo.",
    ),
    (
        Some("it"),
        "Ehm, il, il progetto è pronto.",
        "Il progetto è pronto.",
    ),
    (Some("it"), "È, diciamo, un problema.", "È un problema."),
    (Some("it"), "È un tipo strano.", "È un tipo strano."),
    (Some("it"), "Andiamo piano piano.", "Andiamo piano piano."),
    (
        Some("it"),
        "Comprare il latte. Cancella quello. Comprare le uova.",
        "Comprare le uova.",
    ),
    (
        Some("it"),
        "Chiama Luca, lascia perdere, scrivi a Marco.",
        "Scrivi a Marco.",
    ),
    (
        Some("it"),
        "Lascia stare il gatto.",
        "Lascia stare il gatto.",
    ),
    (
        Some("it"),
        "Ci vediamo alle sei, anzi, alle sette.",
        "Ci vediamo alle sette.",
    ),
    (
        Some("it"),
        "Mandalo a Luca, no, a Marco.",
        "Mandalo a Marco.",
    ),
    (
        Some("it"),
        "Prenota per lunedì, cioè, martedì.",
        "Prenota per martedì.",
    ),
    (Some("it-IT"), "uhm perché perché no", "Perché no."),
    // Regional tags pick the base language; no or an unknown language uses every list.
    (Some("en-US"), "uh yes", "Yes."),
    (None, "um ehm the the plan works", "The plan works."),
    // Whisper writes a spoken "uh" as "ah", so it goes when set off by punctuation, and stays as
    // a word ("ah ha", "the ah of it"). An opening "Ah," is set off too, a cost worth paying.
    (
        Some("en"),
        "so I think we should, ah, ship it",
        "So I think we should ship it.",
    ),
    (
        Some("en"),
        "ah, I see what you mean",
        "I see what you mean.",
    ),
    (Some("en"), "the ah moment", "The ah moment."),
    (Some("de"), "Ehm, das das ist gut.", "Das ist gut."),
    // Empty and whitespace-only input stays empty.
    (Some("en"), "", ""),
    (Some("en"), "  um  ", ""),
];

#[test]
fn golden_cases_clean_to_the_expected_text() {
    let mut failures = Vec::new();
    for &(language, input, expected) in CASES {
        let actual = rule_cleanup(input, language);
        if actual != expected {
            failures.push(format!(
                "[{language:?}] {input:?}\n   expected {expected:?}\n   got      {actual:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn hesitations_go_however_they_are_written() {
    let cases = [
        (
            "Mmmh... oh... vorrei, eeeh, una pizza.",
            "Vorrei una pizza.",
        ),
        ("Scarica ehm...tutti i video.", "Scarica tutti i video."),
        ("Ehm, allora, hmmm, partiamo.", "Allora partiamo."),
        ("Ah, ho capito.", "Ho capito."),
        ("Ohhh, mhm, va bene.", "Va bene."),
    ];
    for (spoken, written) in cases {
        assert_eq!(rule_cleanup(spoken, Some("it")), written, "{spoken}");
    }
}

#[test]
fn a_hesitation_is_not_a_word_that_looks_like_one() {
    let cases = [
        "Oh no, ho perso il treno.",
        "Prendi la taglia M.",
        "Mamma mia, che caldo.",
        "Ho sentito un oh di stupore.",
    ];
    for spoken in cases {
        assert_eq!(rule_cleanup(spoken, Some("it")), spoken);
    }
}

#[test]
fn pauses_become_the_punctuation_they_stand_for() {
    let cases = [
        // The user's own dictation.
        (
            "Scarica tutti i video, trascrivili... cioè, tutti no, ehm...scarica... quelli dell'ultimo mese.",
            "Scarica tutti i video, trascrivili, cioè, tutti no, scarica, quelli dell'ultimo mese.",
        ),
        (
            "Trascrivili e... Scarta quelli.",
            "Trascrivili e scarta quelli.",
        ),
        ("Vorrei parlare con... Marco.", "Vorrei parlare con Marco."),
        ("Ciao... come stai?", "Ciao, come stai?"),
        ("Sta attivo... Misura un po'.", "Sta attivo. Misura un po'."),
        ("E qualche cosa…", "E qualche cosa."),
        ("Davvero...?", "Davvero?"),
        (
            "...trascrivili ...e poi ... basta",
            "Trascrivili, e poi, basta.",
        ),
    ];
    for (spoken, written) in cases {
        assert_eq!(rule_cleanup(spoken, Some("it")), written, "{spoken}");
    }
}
