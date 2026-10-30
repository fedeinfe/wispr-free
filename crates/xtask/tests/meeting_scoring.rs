//! The meeting score is only worth its number if it can fail, so it's tested without a model.

use wf_meetings::{ActionItem, Item, Summary, TranscriptRow};
use xtask::meeting_eval::{
    MeetingScript, PlacedTurn, Truth, score_citations, score_summary, word_error_rate,
};

fn script() -> MeetingScript {
    toml::from_str(
        r#"
        you = "Elena"
        stale = ["november 4"]
        [voices]
        Elena = "Samantha"
        [script]
        turns = [["Elena", "Hello."]]
        [[decisions]]
        id = "date"
        all = [["november 18", "18th of november"], ["ship", "launch", "release"]]
        not = ["november 4"]
        [[actions]]
        id = "plan"
        owner = "Marco"
        task = [["migration"]]
        due = ["friday"]
        [[actions]]
        id = "mine"
        owner = "Elena"
        task = [["email"]]
        due = []
    "#,
    )
    .unwrap()
}

fn item(text: &str) -> Item {
    Item {
        text: text.to_owned(),
        at_ms: None,
    }
}

fn action(owner: Option<&str>, task: &str, due: Option<&str>) -> ActionItem {
    ActionItem {
        task: task.to_owned(),
        owner: owner.map(str::to_owned),
        due: due.map(str::to_owned),
        at_ms: None,
    }
}

fn summary(decisions: &[&str], action_items: Vec<ActionItem>) -> Summary {
    Summary {
        title: None,
        tldr: String::new(),
        key_points: Vec::new(),
        decisions: decisions.iter().map(|text| item(text)).collect(),
        action_items,
        partial: false,
        transcript_hash: String::new(),
    }
}

#[test]
fn a_perfect_summary_scores_everything() {
    let summary = summary(
        &["We ship version three on November 18."],
        vec![
            action(
                Some("Marco"),
                "finish the migration script",
                Some("by Friday"),
            ),
            action(Some("You"), "email the customers", None),
        ],
    );

    let score = score_summary(&script(), &summary);

    assert_eq!(score.decisions_right(), 1);
    assert_eq!(score.actions_found(), 2);
    assert_eq!(score.actions_right(), 2);
    assert!(score.extra_decisions.is_empty());
    assert!(score.extra_actions.is_empty());
}

#[test]
fn an_empty_summary_scores_nothing() {
    let score = score_summary(&script(), &summary(&[], Vec::new()));

    assert_eq!(score.decisions_right(), 0);
    assert_eq!(score.actions_found(), 0);
    assert_eq!(score.actions_right(), 0);
}

#[test]
fn a_decision_needs_every_group() {
    let score = score_summary(
        &script(),
        &summary(&["November 18 is a Tuesday."], Vec::new()),
    );

    assert_eq!(score.decisions_right(), 0);
    assert_eq!(score.extra_decisions.len(), 1);
}

#[test]
fn where_the_discussion_started_is_not_where_it_ended() {
    let score = score_summary(
        &script(),
        &summary(
            &["We ship on November 18, keeping November 4 as well."],
            Vec::new(),
        ),
    );

    assert_eq!(score.decisions_right(), 0);
}

#[test]
fn the_wrong_owner_or_due_date_finds_the_task_but_does_not_get_it_right() {
    let summary = summary(
        &[],
        vec![
            action(Some("Sara"), "finish the migration script", Some("Friday")),
            action(Some("You"), "email the customers", Some("next Monday")),
        ],
    );

    let score = score_summary(&script(), &summary);

    assert_eq!(score.actions_found(), 2);
    assert_eq!(score.actions_right(), 0);
}

#[test]
fn inventing_a_due_date_is_wrong_and_omitting_a_stated_one_is_too() {
    let summary = summary(
        &[],
        vec![action(Some("Marco"), "write the migration", None)],
    );

    assert_eq!(score_summary(&script(), &summary).actions_right(), 0);
}

#[test]
fn the_best_of_several_matching_lines_counts() {
    let summary = summary(
        &[],
        vec![
            action(None, "migration", None),
            action(Some("Marco"), "migration script", Some("Friday")),
        ],
    );

    let score = score_summary(&script(), &summary);

    assert_eq!(score.actions_right(), 1);
    assert!(score.extra_actions.is_empty(), "both lines are about it");
}

#[test]
fn lines_about_nothing_known_are_listed_as_extras() {
    let summary = summary(
        &["We bought a boat."],
        vec![action(Some("Tom"), "paint the boat", None)],
    );

    let score = score_summary(&script(), &summary);

    assert_eq!(score.extra_decisions, ["We bought a boat."]);
    assert_eq!(score.extra_actions, ["paint the boat"]);
}

/// A right line doesn't make up for the wrong one beside it, and the TL;DR is read first.
#[test]
fn an_outcome_the_meeting_took_back_is_counted_even_next_to_the_right_one() {
    let mut summary = summary(
        &["We ship on November 18.", "Keep November 4 on the board."],
        Vec::new(),
    );
    summary.tldr = "Version three ships on November 4.".to_owned();

    let score = score_summary(&script(), &summary);

    assert_eq!(score.decisions_right(), 1);
    assert_eq!(score.stale.len(), 2);
}

#[test]
fn the_bundled_script_has_something_to_score() {
    let script = MeetingScript::bundled().unwrap();

    assert_eq!(script.decisions.len(), 8);
    assert_eq!(script.actions.len(), 10);
    assert!(script.turns().len() > 150);
    for turn in script.turns() {
        assert!(script.voices.contains_key(&turn.0), "{}", turn.0);
    }
}

#[test]
fn word_error_rate_counts_edits_over_the_reference() {
    assert_eq!(word_error_rate("the cat sat", "The cat, sat."), 0.0);
    assert_eq!(word_error_rate("the cat sat down", "the dog sat"), 0.5);
    assert_eq!(word_error_rate("", ""), 0.0);
}

fn fixture_turns() -> (MeetingScript, Truth, Vec<TranscriptRow>) {
    let script = MeetingScript::bundled().unwrap();
    let turn = |who: &str, start_ms, end_ms, text: &str| PlacedTurn {
        who: who.to_owned(),
        start_ms,
        end_ms,
        text: text.to_owned(),
    };
    let truth = Truth {
        length_ms: 1_800_000,
        turns: vec![
            turn(
                "Marco",
                379_000,
                386_000,
                "That is me. I will finish the migration script by Friday.",
            ),
            turn(
                "Tom",
                741_000,
                756_000,
                "I will have the regression test plan by Monday.",
            ),
        ],
    };
    let row = |at_ms, text: &str| TranscriptRow {
        at_ms,
        speaker: None,
        name: "Others".to_owned(),
        text: text.to_owned(),
        gap: false,
    };
    let rows = vec![
        row(100_000, "We talked about the migration a while ago."),
        row(
            380_000,
            "That is me, I will finish the migration script by Friday.",
        ),
        row(742_000, "I'll have the plan by Monday."),
    ];
    (script, truth, rows)
}

#[test]
fn a_citation_on_the_turn_that_took_the_action_is_right() {
    let (script, truth, rows) = fixture_turns();

    let score = score_citations(
        &script,
        &truth,
        "- Marco will finish the migration script by Friday. [06:20]\n\
         - Tom writes the regression test plan by Monday. [12:22]",
        &rows,
    );

    assert_eq!((score.times.len(), score.real(), score.right()), (2, 2, 2));
    assert_eq!(score.covered, ["migration-script", "regression-plan"]);
    assert_eq!(score.known, 10);
}

#[test]
fn a_real_line_about_something_else_is_wrong() {
    let (script, truth, rows) = fixture_turns();
    let score = score_citations(&script, &truth, "Marco does the migration. [01:40]", &rows);
    assert_eq!((score.real(), score.right()), (1, 0));
    assert!(score.covered.is_empty());
    assert!(score.render().contains("WRONG"));
}

#[test]
fn a_time_with_no_line_counts_for_nothing() {
    let (script, truth, rows) = fixture_turns();
    let score = score_citations(&script, &truth, "Marco does the migration. [09:09]", &rows);
    assert_eq!((score.real(), score.right()), (0, 0));
    assert!(score.render().contains("NO LINE"));
}

#[test]
fn a_line_with_no_time_is_counted() {
    let (script, truth, rows) = fixture_turns();
    let score = score_citations(&script, &truth, "Marco does the migration.", &rows);
    assert_eq!(score.bare_lines.len(), 1);
    assert_eq!(score.times.len(), 0);
}

#[test]
fn every_known_action_says_where_it_was_taken_on() {
    let script = MeetingScript::bundled().unwrap();
    for action in &script.actions {
        assert!(!action.said.is_empty(), "{}", action.id);
        let turns = script.turns().iter();
        let found = turns.filter(|(_, text)| text.to_lowercase().contains(&action.said));
        assert_eq!(found.count(), 1, "{}", action.id);
    }
}
