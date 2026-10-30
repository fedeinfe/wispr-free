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
fn items_introduced_with_an_article_count_too() {
    let cases = [
        (
            "Allora senti, devo fixare questi 4 bug: il primo è che non funziona il login, il secondo è che non funziona la pagina home page, la terza è che praticamente la sera è fredda.",
            "Allora senti, devo fixare questi 4 bug:\n1. non funziona il login\n2. non funziona la pagina home page\n3. praticamente la sera è fredda",
        ),
        (
            "Devo fixare questi bug il primo è che non funziona il login, il secondo è che manca il bottone",
            "Devo fixare questi bug:\n1. non funziona il login\n2. manca il bottone",
        ),
        (
            "Ci sono tre problemi: il primo problema è il login, il secondo problema è il bottone, il terzo è la home.",
            "Ci sono tre problemi:\n1. il login\n2. il bottone\n3. la home",
        ),
        (
            "Ho questi due problemi: il primo il login, il secondo il bottone.",
            "Ho questi due problemi:\n1. il login\n2. il bottone",
        ),
        (
            "There are three bugs: the first is that login fails, the second one is that the button is gone, the third is the home page.",
            "There are three bugs:\n1. login fails\n2. the button is gone\n3. the home page",
        ),
        (
            "Ciao ragazzi, come state? Oggi vorrei fare queste tre cose. La prima è andare a fare la spesa, la seconda organizzare tutto e poi la terza andare a mangiare.",
            "Ciao ragazzi, come state? Oggi vorrei fare queste tre cose:\n1. andare a fare la spesa\n2. organizzare tutto\n3. andare a mangiare",
        ),
        (
            "Ho queste tre cose: la prima il login, la seconda il bottone e la terza la home.",
            "Ho queste tre cose:\n1. il login\n2. il bottone\n3. la home",
        ),
        (
            "Ho queste due cose da fare: la prima è il login e poi la seconda è il bottone.",
            "Ho queste due cose da fare:\n1. il login\n2. il bottone",
        ),
        (
            "Fix these three bugs: the first is login, the second is the button, and then the third is the home page.",
            "Fix these three bugs:\n1. login\n2. the button\n3. the home page",
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
        "Il primo è arrivato Marco, il secondo è arrivato Luca, il terzo è arrivato Gino.",
        "Queste tre cose non mi piacciono per niente.",
        "Queste cose succedono, lo so.",
        "Ho due figli, la prima fa il liceo e la seconda l'università.",
        "Il primo piatto era buono e il secondo no.",
        "Ho comprato due biglietti, il primo è per me e il secondo per Anna.",
    ];
    for sentence in cases {
        assert_eq!(format_lists(sentence), None, "{sentence}");
    }
}

#[test]
fn text_with_lines_is_left_as_it_is() {
    assert_eq!(format_lists("Spesa:\n- pane\n- latte"), None);
}
