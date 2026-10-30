//! Learning from a fix, as the running flow does it: when a watch starts and ends, and what keeps
//! it from happening at all. Which fixes count is tested next to the rule.

mod support;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use support::{FN, Rig, base_settings, fixture, wait_for};
use wf_core::dictation::PillInput;
use wf_core::settings::Settings;
use wf_dictation::{Event, LOOK_EVERY, STEADY_LOOKS, WATCH_FOR};
use wf_platform::context::{Focus, TextAround};

const INSERTED: &str = "We deploy on kubernets today.";
const FIXED: &str = "We deploy on kubernetes today.";

fn dictated() -> Rig {
    dictated_with(base_settings())
}

/// A rig right after a dictation of [`INSERTED`], which the field shows.
fn dictated_with(settings: Settings) -> Rig {
    let rig = Rig::new(settings);
    rig.probe.hears(INSERTED);
    rig.dictate(&fixture("speech_en"));
    rig.wait_inserted();
    assert_eq!(rig.inserted(), [INSERTED]);
    rig.field(INSERTED);
    rig
}

fn learned(rig: &Rig) -> Vec<String> {
    rig.settle();
    std::iter::from_fn(|| rig.events.try_recv().ok())
        .filter_map(|event| match event {
            Event::WordLearned(word) => Some(word),
            _ => None,
        })
        .collect()
}

/// Sees the text whole, then watches the fix for as long as it takes.
fn fix(rig: &Rig) {
    rig.look();
    rig.field(FIXED);
    for _ in 0..STEADY_LOOKS {
        rig.look();
    }
}

#[test]
fn a_fix_right_after_a_dictation_is_learned_once() {
    let rig = dictated();

    fix(&rig);

    assert_eq!(learned(&rig), ["kubernetes"]);
}

#[test]
fn the_watch_ends_with_what_it_learned() {
    let rig = dictated();
    fix(&rig);
    assert_eq!(learned(&rig), ["kubernetes"]);
    let reads = rig.context_reads();

    rig.field("We deploy on kubernetes tomorow.");
    rig.hold(LOOK_EVERY * 8);
    rig.settle();

    assert_eq!(rig.context_reads(), reads);
    assert_eq!(learned(&rig), Vec::<String>::new());
}

#[test]
fn with_the_setting_off_the_field_is_never_read_again() {
    let mut settings = base_settings();
    settings.style.auto_add_corrections = false;
    let rig = dictated_with(settings);
    let reads = rig.context_reads();

    rig.field(FIXED);
    rig.hold(LOOK_EVERY);
    rig.hold(LOOK_EVERY);
    rig.settle();

    assert_eq!(rig.context_reads(), reads);
    assert_eq!(learned(&rig), Vec::<String>::new());
}

#[test]
fn with_context_awareness_off_the_field_is_never_read_at_all() {
    let mut settings = base_settings();
    settings.privacy.context_awareness = false;
    let rig = dictated_with(settings);

    rig.hold(LOOK_EVERY);
    rig.hold(LOOK_EVERY);
    rig.settle();

    assert_eq!(rig.context_reads(), 0);
}

#[test]
fn turning_the_setting_off_ends_a_watch_at_once() {
    let rig = dictated();
    rig.look();
    let reads = rig.context_reads();
    let mut settings = base_settings();
    settings.style.auto_add_corrections = false;

    rig.dictation.settings_changed(settings);
    rig.settle();
    rig.field(FIXED);
    rig.hold(LOOK_EVERY);
    rig.settle();

    assert_eq!(rig.context_reads(), reads);
}

#[test]
fn turning_context_awareness_off_ends_a_watch_at_once() {
    let rig = dictated();
    rig.look();
    let reads = rig.context_reads();
    let mut settings = base_settings();
    settings.privacy.context_awareness = false;

    rig.dictation.settings_changed(settings);
    rig.settle();
    rig.hold(LOOK_EVERY);
    rig.settle();

    assert_eq!(rig.context_reads(), reads);
}

#[test]
fn turning_the_setting_on_applies_to_the_next_dictation() {
    let mut settings = base_settings();
    settings.style.auto_add_corrections = false;
    let rig = dictated_with(settings);

    rig.dictation.settings_changed(base_settings());
    rig.dictate(&fixture("speech_en"));
    rig.wait_inserted();
    fix(&rig);

    assert_eq!(learned(&rig), ["kubernetes"]);
}

#[test]
fn a_fix_after_the_window_is_too_late() {
    let rig = dictated();
    rig.look();

    rig.clock.advance(WATCH_FOR);
    rig.dictation.poke();
    rig.settle();
    let reads = rig.context_reads();
    rig.field(FIXED);
    rig.hold(LOOK_EVERY);
    rig.hold(LOOK_EVERY);
    rig.settle();

    assert_eq!(rig.context_reads(), reads);
    assert_eq!(learned(&rig), Vec::<String>::new());
}

#[test]
fn a_new_dictation_ends_the_watch() {
    let rig = dictated();
    rig.look();
    rig.field(FIXED);
    rig.look();

    rig.press(FN);
    rig.mic.wait_open();
    rig.settle();
    let reads = rig.context_reads();
    rig.hold(LOOK_EVERY);
    rig.hold(LOOK_EVERY);
    rig.settle();

    assert_eq!(rig.context_reads(), reads);
}

#[test]
fn a_secure_field_ends_the_watch() {
    let rig = dictated();
    rig.look();
    rig.context.0.lock().unwrap().focus = Focus::Secure;
    rig.look();
    let reads = rig.context_reads();

    rig.hold(LOOK_EVERY);
    rig.settle();

    assert_eq!(rig.context_reads(), reads);
    assert_eq!(learned(&rig), Vec::<String>::new());
}

#[test]
fn text_that_only_reached_the_clipboard_is_not_watched() {
    let rig = Rig::new(base_settings());
    rig.probe.hears(INSERTED);
    rig.log.no_focus.store(true, Ordering::SeqCst);

    rig.dictate(&fixture("speech_en"));
    rig.wait_inserted();
    rig.settle();
    let reads = rig.context_reads();
    rig.hold(LOOK_EVERY);
    rig.hold(LOOK_EVERY);
    rig.settle();

    assert_eq!(rig.context_reads(), reads);
}

#[test]
fn text_sent_with_enter_is_not_watched() {
    let mut settings = base_settings();
    settings.style.press_enter_command = true;
    let rig = Rig::new(settings);
    rig.probe.hears("We deploy on kubernets today, press enter");

    rig.dictate(&fixture("speech_en"));
    rig.wait_inserted();
    wait_for("the Enter", || rig.log.enters.load(Ordering::SeqCst) == 1);
    rig.settle();
    let reads = rig.context_reads();
    rig.hold(LOOK_EVERY);
    rig.hold(LOOK_EVERY);
    rig.settle();

    assert_eq!(rig.context_reads(), reads);
}

#[test]
fn pasting_an_old_transcript_is_not_watched() {
    let rig = dictated();
    fix(&rig);
    assert_eq!(learned(&rig), ["kubernetes"]);

    rig.field(INSERTED);
    rig.dictation.paste_last();
    wait_for("the paste", || rig.inserted().len() == 2);
    rig.settle();
    let reads = rig.context_reads();
    rig.hold(LOOK_EVERY);
    rig.hold(LOOK_EVERY);
    rig.settle();

    assert_eq!(rig.context_reads(), reads);
}

#[test]
fn a_trial_is_not_watched() {
    let rig = Rig::new(base_settings());
    rig.probe.hears(INSERTED);

    rig.dictation.start_trial();
    rig.mic.wait_open();
    rig.mic.play(&fixture("speech_en"));
    rig.dictation.on_pill(PillInput::Stop);
    rig.wait_event("the trial", |event| {
        matches!(event, Event::Trial(_)).then_some(())
    });
    rig.settle();
    let reads = rig.context_reads();
    rig.hold(LOOK_EVERY);
    rig.hold(LOOK_EVERY);
    rig.settle();

    assert_eq!(rig.context_reads(), reads);
}

#[test]
fn a_watch_never_holds_up_the_next_dictation() {
    let rig = dictated();
    rig.look();
    let context = Arc::clone(&rig.context);
    // A read of the field now hangs, like one in an app that stopped answering.
    let _stuck = context.0.lock().unwrap();

    rig.hold(LOOK_EVERY);
    rig.settle();
    rig.press(FN);

    rig.mic.wait_open();
}

#[test]
fn the_text_around_the_cursor_is_read_the_way_context_gives_it() {
    let rig = dictated();
    rig.look();
    rig.context.0.lock().unwrap().text =
        Some(TextAround::new("We deploy on kuber", "", "netes today."));

    for _ in 0..STEADY_LOOKS {
        rig.look();
    }

    assert_eq!(learned(&rig), ["kubernetes"]);
}
