//! The meeting score is only worth its number if it can fail, so it's tested without a model.

use wf_meetings::{ActionItem, Item, Summary};
use xtask::meeting_eval::{MeetingScript, score_summary, word_error_rate};

fn script() -> MeetingScript {
    toml::from_str(
        r#"
        you = "Elena"
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
