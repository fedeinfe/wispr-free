//! The eval is only worth its number if it can fail, so its scoring is tested without a model.

use wf_pipeline::rules::rule_cleanup;
use wf_pipeline::{Llm, LlmPrompt, PipelineError};
use xtask::eval::{Case, RulesOnly, cases, run, score};

fn case(toml: &str) -> Case {
    toml::from_str(toml).unwrap()
}

fn passes(case: &Case, output: &str) -> bool {
    score(case, output).is_empty()
}

#[test]
fn case_and_punctuation_alone_do_not_fail_a_reference_match() {
    let case = case(
        r#"
        id = "x"
        category = "plain"
        input = "x"
        expect = ["We ship on Monday."]
    "#,
    );

    assert!(passes(&case, "We ship on Monday!"));
    assert!(passes(&case, "We, ship on monday."));
}

#[test]
fn different_words_fail() {
    let case = case(
        r#"
        id = "x"
        category = "plain"
        input = "x"
        expect = ["We ship on Monday."]
    "#,
    );

    assert!(!passes(&case, "We ship on Friday."));
}

#[test]
fn a_long_sentence_tolerates_one_word_in_ten() {
    let case = case(
        r#"
        id = "x"
        category = "plain"
        input = "x"
        expect = ["One two three four five six seven eight nine ten."]
    "#,
    );

    assert!(passes(
        &case,
        "One two three four five six seven eight nine eleven."
    ));
    assert!(!passes(
        &case,
        "One two three four five six seven eight twelve eleven."
    ));
}

#[test]
fn any_reference_may_match() {
    let case = case(
        r#"
        id = "x"
        category = "number"
        input = "x"
        expect = ["It costs $25.", "It costs 25 dollars."]
    "#,
    );

    assert!(passes(&case, "It costs 25 dollars."));
    assert!(passes(&case, "It costs $25."));
}

#[test]
fn an_output_must_start_capitalized_and_end_a_sentence_by_default() {
    let case = case(
        r#"
        id = "x"
        category = "plain"
        input = "x"
        expect = ["We ship on Monday."]
    "#,
    );

    assert!(!passes(&case, "we ship on Monday."));
    assert!(!passes(&case, "We ship on Monday"));
}

#[test]
fn contains_and_absent_are_case_sensitive_substrings() {
    let case = case(
        r#"
        id = "x"
        category = "self-correction"
        input = "x"
        expect = ["Book it for Friday."]
        contains = ["Friday"]
        absent = ["Monday"]
    "#,
    );

    assert!(passes(&case, "Book it for Friday."));
    assert!(!passes(&case, "Book it for friday."));
    assert!(!score(&case, "Book it for Friday, not Monday.").is_empty());
}

#[test]
fn list_items_counts_marked_lines_and_markers_do_not_count_as_words() {
    let case = case(
        r#"
        id = "x"
        category = "list"
        input = "x"
        expect = ["Groceries: milk eggs"]
        list_items = 2
    "#,
    );

    assert!(passes(&case, "Groceries:\n- milk\n- eggs"));
    assert!(passes(&case, "Groceries:\n1. milk\n2. eggs"));
    assert!(!passes(&case, "Groceries: milk, eggs."));
}

#[test]
fn list_items_of_zero_rejects_a_list() {
    let case = case(
        r#"
        id = "x"
        category = "list"
        input = "x"
        expect = ["I bought milk eggs."]
        list_items = 0
    "#,
    );

    assert!(passes(&case, "I bought milk, eggs."));
    assert!(!passes(&case, "I bought:\n- milk\n- eggs"));
}

#[test]
fn styles_check_casing_and_the_final_period() {
    let case = case(
        r#"
        id = "x"
        category = "style"
        input = "x"
        expect = ["see you at the gym"]
        lowercase = true
        final_period = false
    "#,
    );

    assert!(passes(&case, "see you at the gym"));
    assert!(!passes(&case, "see you at the gym."));
    assert!(!passes(&case, "See you at the gym"));
}

#[test]
fn the_shipped_set_meets_the_ticket_and_every_category_is_there() {
    let cases = cases().unwrap();

    assert!(cases.len() >= 50, "{} cases", cases.len());
    for category in ["self-correction", "list", "number", "style"] {
        assert!(
            cases.iter().filter(|c| c.category == category).count() >= 10,
            "{category}"
        );
    }
    let mut ids: Vec<_> = cases.iter().map(|c| c.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), cases.len(), "duplicate ids");
}

/// Replies with the case's first reference.
struct Oracle;

impl Llm for Oracle {
    fn generate(
        &mut self,
        prompt: &LlmPrompt,
        _on_token: &mut dyn FnMut(&str),
    ) -> Result<String, PipelineError> {
        let mut lines = prompt.user.lines();
        let transcript = lines
            .find(|l| *l == "<transcript>")
            .and(lines.next())
            .unwrap();
        let cases = cases().unwrap();
        let case = cases
            .iter()
            .find(|c| rule_cleanup(&c.input, c.language.as_deref()) == transcript)
            .unwrap();
        Ok(case.expect[0].clone())
    }
}

#[test]
fn a_perfect_model_scores_full_marks() {
    let cases = cases().unwrap();

    let report = run(&cases, &mut Oracle);

    let failed: Vec<_> = report
        .failures()
        .map(|r| (&r.id, &r.misses, &r.fallback))
        .collect();
    assert!(failed.is_empty(), "{failed:#?}");
    assert_eq!(report.passed(), cases.len());
}

#[test]
fn rule_cleanup_alone_scores_well_below_the_bar() {
    let cases = cases().unwrap();

    let report = run(&cases, &mut RulesOnly);

    assert!(
        report.score() < 0.6,
        "the eval cannot tell the LLM from rules: {}",
        report.score()
    );
}
