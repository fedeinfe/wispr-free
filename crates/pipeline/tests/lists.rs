//! Spoken lists: expected outputs are hand-written, and the first cases are the user's own.

use wf_pipeline::lists::format_lists;
use wf_pipeline::rules::rule_cleanup;

#[test]
fn a_counted_announcement_becomes_a_bulleted_list() {
    let cases = [
        (
            "oggi devo fare queste tre cose andare a fare la spesa, cucinare, pulire",
            "Oggi devo fare queste tre cose:\n- Fare la spesa\n- Cucinare\n- Pulire",
        ),
        (
            "Oggi devo fare queste tre cose: andare a fare la spesa, cucinare e pulire.",
            "Oggi devo fare queste tre cose:\n- Fare la spesa\n- Cucinare\n- Pulire",
        ),
        (
            "Today I need to do these three things, buy milk, call Anna and clean the garage.",
            "Today I need to do these three things:\n- Buy milk\n- Call Anna\n- Clean the garage",
        ),
        (
            "Oggi devo fare queste tre cose. Andare a fare la spesa, cucinare, pulire.",
            "Oggi devo fare queste tre cose:\n- Fare la spesa\n- Cucinare\n- Pulire",
        ),
        (
            "Oggi devo fare queste tre cose. Andare a fare la spesa. Cucinare. Pulire.",
            "Oggi devo fare queste tre cose:\n- Fare la spesa\n- Cucinare\n- Pulire",
        ),
        (
            "Oggi devo fare queste 3 cose, andare a Roma, cucinare, pulire.",
            "Oggi devo fare queste 3 cose:\n- Andare a Roma\n- Cucinare\n- Pulire",
        ),
        (
            "Mi servono le seguenti cose: pane, latte, uova.",
            "Mi servono le seguenti cose:\n- Pane\n- Latte\n- Uova",
        ),
        (
            "Devo fare tre cose: pagare l'affitto, chiamare Anna, stirare.",
            "Devo fare tre cose:\n- Pagare l'affitto\n- Chiamare Anna\n- Stirare",
        ),
    ];
    for (spoken, written) in cases {
        let cleaned = rule_cleanup(spoken, Some("it"));
        assert_eq!(format_lists(&cleaned).as_deref(), Some(written), "{spoken}");
    }
}

#[test]
fn counted_off_items_become_a_numbered_list() {
    let cases = [
        (
            "Risolvi questi quattro bug primo non funziona il login, secondo non funziona il bottone, terzo non carica",
            "Risolvi questi quattro bug:\n1. non funziona il login\n2. non funziona il bottone\n3. non carica",
        ),
        (
            "Risolvi questi bug: primo, non funziona il login. Secondo, non funziona il bottone. Terzo, non carica.",
            "Risolvi questi bug:\n1. non funziona il login\n2. non funziona il bottone\n3. non carica",
        ),
        (
            "Ci sono due problemi, prima cosa il login, seconda cosa il bottone.",
            "Ci sono due problemi:\n1. il login\n2. il bottone",
        ),
        (
            "In primo luogo il login, in secondo luogo il bottone, in terzo luogo la home.",
            "1. il login\n2. il bottone\n3. la home",
        ),
        (
            "Fix these bugs, first the login, second the button, and third the home page.",
            "Fix these bugs:\n1. the login\n2. the button\n3. the home page",
        ),
        (
            "Da fare: punto uno il login, punto due il bottone, punto tre la home.",
            "Da fare:\n1. il login\n2. il bottone\n3. la home",
        ),
        (
            "Da fare: 1. il login 2. il bottone 3. la home",
            "Da fare:\n1. Il login\n2. Il bottone\n3. La home",
        ),
    ];
    for (spoken, written) in cases {
        let cleaned = rule_cleanup(spoken, Some("it"));
        assert_eq!(format_lists(&cleaned).as_deref(), Some(written), "{spoken}");
    }
}

#[test]
fn what_follows_the_list_stays_after_it() {
    assert_eq!(
        format_lists(
            "Controlla: primo il login, secondo il bottone, terzo la home. Poi fammi sapere."
        )
        .as_deref(),
        Some("Controlla:\n1. il login\n2. il bottone\n3. la home\n\nPoi fammi sapere."),
    );
}

#[test]
fn ordinary_sentences_stay_sentences() {
    let cases = [
        "Secondo me il primo tempo è andato bene.",
        "Il primo piatto era buono, il secondo no, il terzo non l'ho assaggiato.",
        "Prima di uscire chiama Anna, secondo me è a casa.",
        "Arrivo tra un secondo.",
        "Ho visto questi due ragazzi, Marco e Luca, sono venuti ieri.",
        "Oggi ho comprato pane, latte e uova.",
        "Ho comprato tre mele, due pere, un kiwi.",
        "He came first, second place went to Anna.",
        "Queste tre cose non mi piacciono per niente.",
        "Queste cose succedono, lo so.",
    ];
    for sentence in cases {
        assert_eq!(format_lists(sentence), None, "{sentence}");
    }
}

#[test]
fn text_with_lines_is_left_as_it_is() {
    assert_eq!(format_lists("Spesa:\n- pane\n- latte"), None);
}
