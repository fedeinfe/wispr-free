//! Local passes (spec §3): the deterministic steps between cleanup and insertion. Expected
//! outputs are hand-written from the spec, not derived from code.

use wf_pipeline::passes::{
    AppCategory, LocalPassOptions, Replacement, Snippet, Style, app_category, apply_replacements,
    apply_style, break_after_greeting, drop_chat_period, expand_snippets, local_passes,
    match_casing, match_spacing, strip_press_enter, whole_snippet,
};
use wf_pipeline::rules::rule_cleanup;

#[test]
fn styles_rewrite_the_sentence_case_and_final_period_of_rule_cleanup() {
    let cases = [
        (
            Style::Formal,
            "Ok, see you at noon.",
            "Ok, see you at noon.",
        ),
        (Style::Casual, "Ok, see you at noon.", "Ok, see you at noon"),
        (
            Style::VeryCasual,
            "Ok, see you at noon.",
            "ok, see you at noon",
        ),
        (
            Style::Excited,
            "Ok, see you at noon.",
            "Ok, see you at noon!",
        ),
        // Casual keeps a comma that lists or separates, and drops the one before a conjunction.
        (
            Style::Casual,
            "We shipped, and it works.",
            "We shipped and it works",
        ),
        (
            Style::Casual,
            "Red, green, and blue.",
            "Red, green and blue",
        ),
        (
            Style::Casual,
            "Siamo pronti, ma aspettiamo.",
            "Siamo pronti ma aspettiamo",
        ),
        // Only the very last period goes; other sentences and other marks stay.
        (Style::Casual, "Done. See you.", "Done. See you"),
        (
            Style::VeryCasual,
            "Is It Done? I Think So.",
            "is it done? i think so",
        ),
        (Style::Casual, "Are we done?", "Are we done?"),
        (Style::Excited, "Are we done?", "Are we done?"),
        (
            Style::Excited,
            "Great news. We shipped.",
            "Great news. We shipped!",
        ),
        (Style::Casual, "Wait for it...", "Wait for it..."),
        (Style::Excited, "Wait for it...", "Wait for it..."),
        (
            Style::VeryCasual,
            "Line one.\nLine two.",
            "line one.\nline two",
        ),
    ];
    for (style, input, expected) in cases {
        assert_eq!(
            apply_style(input, style),
            expected,
            "{style:?} on {input:?}"
        );
    }
}

#[test]
fn every_style_is_idempotent_so_it_can_follow_llm_cleanup() {
    let styles = [
        Style::Formal,
        Style::Casual,
        Style::VeryCasual,
        Style::Excited,
    ];
    for style in styles {
        let once = apply_style("Sure, and then we go. Ok.", style);
        assert_eq!(apply_style(&once, style), once, "{style:?}");
    }
}

#[test]
fn email_puts_a_line_break_after_the_greeting() {
    let cases = [
        (
            "Hi Marco, thanks for the update.",
            "Hi Marco,\nThanks for the update.",
        ),
        (
            "Dear Ms. Rossi, please find it attached.",
            "Dear Ms. Rossi,\nPlease find it attached.",
        ),
        (
            "Good morning team, the build is green.",
            "Good morning team,\nThe build is green.",
        ),
        (
            "Ciao Anna, ci vediamo domani.",
            "Ciao Anna,\nCi vediamo domani.",
        ),
        (
            "Buongiorno: allego il file.",
            "Buongiorno:\nAllego il file.",
        ),
        // Nothing follows the greeting, or a break is already there.
        ("Hi Marco,", "Hi Marco,"),
        ("Hi Marco,\nThanks.", "Hi Marco,\nThanks."),
        // Not a greeting.
        ("Thanks Marco, see you soon.", "Thanks Marco, see you soon."),
        (
            "Hi there no comma in this one.",
            "Hi there no comma in this one.",
        ),
        // A comma this far in is not closing a greeting.
        (
            "Hello everyone in the office and in the other team today, we ship.",
            "Hello everyone in the office and in the other team today, we ship.",
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(break_after_greeting(input), expected, "{input:?}");
    }
}

#[test]
fn short_chat_messages_drop_the_final_period() {
    let cases = [
        ("Sounds good.", "Sounds good"),
        ("Sounds good", "Sounds good"),
        ("Sounds good!", "Sounds good!"),
        ("Are you coming?", "Are you coming?"),
        ("On my way...", "On my way..."),
        // More than one sentence or line reads as a real message, so it stays finished.
        ("Sounds good. See you there.", "Sounds good. See you there."),
        ("Sounds good.\nThanks.", "Sounds good.\nThanks."),
        (
            "This one has quite a few more words than a short chat message would ever have.",
            "This one has quite a few more words than a short chat message would ever have.",
        ),
        ("", ""),
    ];
    for (input, expected) in cases {
        assert_eq!(drop_chat_period(input), expected, "{input:?}");
    }
}

#[test]
fn a_trailing_press_enter_command_is_cut_and_reported() {
    let cases = [
        ("Send it, press enter.", Some("Send it.")),
        ("Thanks press enter", Some("Thanks")),
        ("Thanks. Press enter.", Some("Thanks.")),
        ("Thanks! Press Enter", Some("Thanks!")),
        ("Are you there, press enter?", Some("Are you there?")),
        (
            "Ci vediamo domani, premi invio.",
            Some("Ci vediamo domani."),
        ),
        ("Press enter.", Some("")),
        // Not at the end, or only part of a longer word.
        ("Press enter to continue.", None),
        ("I always compress enter.", None),
        ("Hello there.", None),
        ("", None),
    ];
    for (input, expected) in cases {
        assert_eq!(strip_press_enter(input).as_deref(), expected, "{input:?}");
    }
}

fn snippet(trigger: &str, text: &str) -> Snippet {
    Snippet {
        trigger: trigger.to_owned(),
        text: text.to_owned(),
    }
}

fn replacement(wrong: &str, right: &str) -> Replacement {
    Replacement {
        wrong: wrong.to_owned(),
        right: right.to_owned(),
    }
}

#[test]
fn a_dictation_that_is_only_a_trigger_is_the_whole_snippet() {
    let snippets = [
        snippet("my email", "fede@example.com"),
        snippet("sign off", "Best,\nFede"),
    ];
    for input in [
        "My email.",
        "my email",
        "MY EMAIL!",
        "  my   email  ",
        "Sign off.",
    ] {
        assert!(whole_snippet(input, &snippets).is_some(), "{input:?}");
    }
    assert_eq!(whole_snippet("Sign off.", &snippets), Some("Best,\nFede"));
    for input in ["Send my email.", "My email address.", ""] {
        assert_eq!(whole_snippet(input, &snippets), None, "{input:?}");
    }
}

#[test]
fn triggers_inside_a_sentence_expand_on_word_boundaries() {
    let snippets = [
        snippet("my email", "fede@example.com"),
        snippet("my work email", "fede@work.example"),
        snippet("sign off", "Best, Fede."),
        snippet("", "never"),
    ];
    let cases = [
        (
            "Write to my email please.",
            "Write to fede@example.com please.",
        ),
        ("Write to My Email.", "Write to fede@example.com."),
        // The longest trigger wins, and every occurrence expands.
        ("Use my work email.", "Use fede@work.example."),
        (
            "My email and my email.",
            "fede@example.com and fede@example.com.",
        ),
        // No expansion inside a longer word, and an expansion's own mark is not doubled.
        ("Dummy email is fine.", "Dummy email is fine."),
        ("Thanks, sign off.", "Thanks, Best, Fede."),
        ("No trigger here.", "No trigger here."),
    ];
    for (input, expected) in cases {
        assert_eq!(expand_snippets(input, &snippets), expected, "{input:?}");
    }
}

#[test]
fn expansions_are_not_scanned_again() {
    let snippets = [snippet("alpha", "beta"), snippet("beta", "gamma")];
    assert_eq!(expand_snippets("alpha beta", &snippets), "beta gamma");
}

#[test]
fn replacements_fix_words_and_keep_the_dictionary_spelling() {
    let rules = [
        replacement("teh", "the"),
        replacement("github", "GitHub"),
        replacement("wispr flow", "Wispr Flow"),
        replacement("perche", "perché"),
    ];
    let cases = [
        ("Push to github now.", "Push to GitHub now."),
        ("Push to GITHUB now.", "Push to GitHub now."),
        ("I use wispr flow daily.", "I use Wispr Flow daily."),
        // A sentence start survives an all-lowercase right side...
        ("Teh cat sat on teh mat.", "The cat sat on the mat."),
        ("Perche no.", "Perché no."),
        // ...and a spelling with its own capitals is exact.
        ("Github is down.", "GitHub is down."),
        // Word boundaries.
        ("Tehran and gethub stay.", "Tehran and gethub stay."),
    ];
    for (input, expected) in cases {
        assert_eq!(apply_replacements(input, &rules), expected, "{input:?}");
    }
}

#[test]
fn replacements_run_once_so_two_can_swap() {
    let rules = [replacement("left", "right"), replacement("right", "left")];
    assert_eq!(apply_replacements("left right", &rules), "right left");
}

#[test]
fn casing_follows_the_text_before_the_cursor() {
    let cases = [
        // Unknown context, or the start of a field or sentence: leave it alone.
        (None, "We left.", "We left."),
        (Some(""), "We left.", "We left."),
        (Some("Done. "), "We left.", "We left."),
        (Some("Done!"), "We left.", "We left."),
        (Some("Is it done?\n"), "We left.", "We left."),
        (Some("He said \"go.\" "), "We left.", "We left."),
        (Some("Done.\n- "), "We left.", "We left."),
        (Some("Done. "), "we left", "we left"),
        // Mid-sentence: the sentence capital from cleanup goes.
        (Some("and then "), "We left.", "we left."),
        (Some("Hi, "), "We left.", "we left."),
        (Some("Note: "), "We left.", "we left."),
        (Some("and then "), "(We left.)", "(we left.)"),
        (Some("Mi hanno detto "), "È tardi.", "è tardi."),
        // Words that keep their capital anywhere.
        (Some("and then "), "I left.", "I left."),
        (Some("and then "), "I'm leaving.", "I'm leaving."),
        (Some("and then "), "NASA called.", "NASA called."),
        (Some("and then "), "GitHub broke.", "GitHub broke."),
    ];
    for (before, input, expected) in cases {
        assert_eq!(
            match_casing(input, before),
            expected,
            "{before:?} + {input:?}"
        );
    }
}

#[test]
fn spacing_fits_the_text_around_the_cursor() {
    let cases = [
        (None, None, "world", "world"),
        (Some(""), None, "world", "world"),
        (Some("Hello"), None, "world", " world"),
        (Some("Hello,"), None, "world", " world"),
        (Some("Done."), None, "Next", " Next"),
        (Some("Hello "), None, "world", "world"),
        (Some("Hello\n"), None, "world", "world"),
        (Some("Hello "), None, " world", "world"),
        // Nothing goes in after an opener or an elision, or before punctuation.
        (Some("("), None, "world", "world"),
        (Some("l'"), None, "amico", "amico"),
        (Some("dell’"), None, "amico", "amico"),
        (Some("Hello"), None, ", world", ", world"),
        // No doubled punctuation on either side.
        (Some("Hello,"), None, ", world", " world"),
        (None, Some(". And more"), "Done.", "Done"),
        (None, Some(", right?"), "Done.", "Done"),
        (None, Some("?"), "Really!", "Really"),
        (None, Some(" and more"), "Done.", "Done."),
        (None, Some(""), "Done.", "Done."),
        (Some("Hello"), Some("."), "world.", " world"),
        (Some("Hello"), None, "", ""),
    ];
    for (before, after, input, expected) in cases {
        assert_eq!(
            match_spacing(input, before, after),
            expected,
            "{before:?} + {input:?} + {after:?}"
        );
    }
}

#[test]
fn the_app_picks_the_category() {
    use AppCategory::{Email, Other, PersonalMessages, WorkMessages};
    let cases = [
        (Some("com.tinyspeck.slackmacgap"), None, WorkMessages),
        (Some("com.microsoft.teams2"), None, WorkMessages),
        (Some("ms-teams"), None, WorkMessages),
        (Some("net.whatsapp.WhatsApp"), None, PersonalMessages),
        (Some("com.apple.MobileSMS"), None, PersonalMessages),
        (Some("org.telegram.desktop"), None, PersonalMessages),
        (Some("com.apple.mail"), None, Email),
        (Some("OUTLOOK"), None, Email),
        (Some("org.mozilla.thunderbird"), None, Email),
        (Some("com.apple.TextEdit"), None, Other),
        (None, None, Other),
        (None, Some("Gmail"), Other),
        // A browser goes by its tab, and only a browser: a document can be named anything.
        (
            Some("com.google.Chrome"),
            Some("Inbox (3) - me@gmail.com - Gmail"),
            Email,
        ),
        (Some("firefox"), Some("Slack | general"), WorkMessages),
        (Some("com.apple.Safari"), Some("WhatsApp"), PersonalMessages),
        (Some("com.google.Chrome"), Some("Recipes"), Other),
        (Some("com.apple.TextEdit"), Some("Gmail ideas"), Other),
    ];
    for (app, title, expected) in cases {
        assert_eq!(app_category(app, title), expected, "{app:?} / {title:?}");
    }
}

fn options<'a>() -> LocalPassOptions<'a> {
    LocalPassOptions {
        style: Style::Formal,
        category: AppCategory::Other,
        snippets: &[],
        replacements: &[],
        press_enter_command: false,
        before: None,
        after: None,
    }
}

/// Rule cleanup then local passes, as with LLM cleanup off.
fn run(asr: &str, options: &LocalPassOptions) -> (String, bool) {
    let output = local_passes(&rule_cleanup(asr, Some("en")), options);
    (output.text, output.press_enter)
}

const SHIP_IT: &str = "um so I I think we should uh ship it";

#[test]
fn rule_cleanup_output_passes_through_formal_other_apps_unchanged() {
    let (text, _) = run(SHIP_IT, &options());
    assert_eq!(text, "So I think we should ship it.");
}

#[test]
fn styles_undo_what_rule_cleanup_always_adds() {
    let casual = LocalPassOptions {
        style: Style::Casual,
        ..options()
    };
    assert_eq!(run(SHIP_IT, &casual).0, "So I think we should ship it");
    let very_casual = LocalPassOptions {
        style: Style::VeryCasual,
        ..options()
    };
    assert_eq!(run(SHIP_IT, &very_casual).0, "so i think we should ship it");
    let excited = LocalPassOptions {
        style: Style::Excited,
        ..options()
    };
    assert_eq!(run(SHIP_IT, &excited).0, "So I think we should ship it!");
}

#[test]
fn only_chat_apps_drop_the_period_of_a_short_message() {
    for category in [AppCategory::PersonalMessages, AppCategory::WorkMessages] {
        let chat = LocalPassOptions {
            category,
            ..options()
        };
        assert_eq!(run("sounds good", &chat).0, "Sounds good");
    }
    for category in [AppCategory::Email, AppCategory::Other] {
        let other = LocalPassOptions {
            category,
            ..options()
        };
        assert_eq!(run("sounds good", &other).0, "Sounds good.");
    }
}

#[test]
fn only_email_breaks_after_the_greeting() {
    let dictated = "hi Marco, thanks for the update";
    let email = LocalPassOptions {
        category: AppCategory::Email,
        ..options()
    };
    assert_eq!(run(dictated, &email).0, "Hi Marco,\nThanks for the update.");
    assert_eq!(
        run(dictated, &options()).0,
        "Hi Marco, thanks for the update."
    );
    let lowercase = LocalPassOptions {
        style: Style::VeryCasual,
        ..email
    };
    assert_eq!(
        run(dictated, &lowercase).0,
        "hi marco,\nthanks for the update"
    );
}

#[test]
fn a_whole_snippet_is_written_verbatim_whatever_the_style() {
    let snippets = [snippet("sign off", "Best,\nFede.")];
    let chat = LocalPassOptions {
        style: Style::VeryCasual,
        category: AppCategory::PersonalMessages,
        snippets: &snippets,
        before: Some("and then "),
        ..options()
    };
    assert_eq!(run("um sign off", &chat).0, "Best,\nFede.");
}

#[test]
fn the_style_never_touches_dictionary_spellings_or_inline_expansions() {
    let snippets = [snippet("my email", "Fede@Example.com")];
    let replacements = [replacement("github", "GitHub"), replacement("fede", "Fede")];
    let very_casual = LocalPassOptions {
        style: Style::VeryCasual,
        snippets: &snippets,
        replacements: &replacements,
        ..options()
    };
    assert_eq!(
        run("Tell Fede on github or at my email", &very_casual).0,
        "tell Fede on GitHub or at Fede@Example.com"
    );
}

#[test]
fn casing_never_touches_dictionary_spellings() {
    let replacements = [replacement("marco", "Marco")];
    let mid_sentence = LocalPassOptions {
        replacements: &replacements,
        before: Some("and then "),
        ..options()
    };
    assert_eq!(run("Marco left", &mid_sentence).0, "Marco left.");
    assert_eq!(
        run(
            "Sara left",
            &LocalPassOptions {
                before: Some("and then "),
                ..options()
            }
        )
        .0,
        "sara left."
    );
}

#[test]
fn the_press_enter_command_is_opt_in_and_works_with_snippets() {
    let snippets = [snippet("my email", "fede@example.com")];
    let on = LocalPassOptions {
        snippets: &snippets,
        press_enter_command: true,
        ..options()
    };
    let off = LocalPassOptions {
        press_enter_command: false,
        ..on
    };
    assert_eq!(
        run("send it press enter", &on),
        ("Send it.".to_owned(), true)
    );
    assert_eq!(
        run("send it press enter", &off),
        ("Send it press enter.".to_owned(), false)
    );
    assert_eq!(
        run("my email press enter", &on),
        ("fede@example.com".to_owned(), true)
    );
    assert_eq!(run("press enter", &on), (String::new(), true));
}

#[test]
fn the_result_fits_between_the_text_around_the_cursor() {
    let inside = LocalPassOptions {
        before: Some("I think"),
        after: Some(". And more"),
        ..options()
    };
    assert_eq!(run("We should ship", &inside).0, " we should ship");
}
