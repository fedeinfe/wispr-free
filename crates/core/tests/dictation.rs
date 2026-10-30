//! Every timing rule of spec §2, driven through the pure state machine with injected time.

use std::time::Duration;

use wf_core::dictation::{CancelReason, Command, DictationMachine, Mode};
use wf_core::shortcut::{Bindings, Combo, Key, KeyEvent, Platform};

/// One scripted input at `t` milliseconds.
#[derive(Clone, Copy)]
enum Step {
    Down(Key),
    Up(Key),
    Tick,
}
use Step::{Down, Tick, Up};

type Script<'a> = &'a [(u64, Step)];

/// A named script and the commands it must produce.
type Case<'a> = (&'a str, Script<'a>, &'a [(u64, Command)]);

const FN: Key = Key::Fn;

/// Runs `script` against the macOS defaults and returns every command with the time it fired.
fn run(script: &[(u64, Step)]) -> Vec<(u64, Command)> {
    let mut machine = DictationMachine::new(Bindings::default_for(Platform::MacOs));
    let mut out = Vec::new();
    for &(ms, step) in script {
        let now = Duration::from_millis(ms);
        let commands = match step {
            Down(key) => machine.on_key(KeyEvent::press(key), now).commands,
            Up(key) => machine.on_key(KeyEvent::release(key), now).commands,
            Tick => machine.tick(now),
        };
        out.extend(commands.into_iter().map(|c| (ms, c)));
    }
    out
}

fn check(cases: &[Case]) {
    for (name, script, expected) in cases {
        assert_eq!(run(script), *expected, "{name}");
    }
}

#[test]
fn hold_then_release_stops_and_inserts() {
    let cases: &[Case] = &[
        (
            "held 800 ms",
            &[(0, Down(FN)), (800, Up(FN))],
            &[
                (0, Command::Start(Mode::PushToTalk)),
                (800, Command::StopAndInsert),
            ],
        ),
        (
            "held exactly 500 ms",
            &[(0, Down(FN)), (500, Up(FN))],
            &[
                (0, Command::Start(Mode::PushToTalk)),
                (500, Command::StopAndInsert),
            ],
        ),
    ];
    check(cases);
}

#[test]
fn quick_tap_with_no_second_press_is_discarded_once_the_window_closes() {
    let cases: &[Case] = &[
        (
            "tap of 200 ms, window still open at 699 ms",
            &[(0, Down(FN)), (200, Up(FN)), (699, Tick)],
            &[(0, Command::Start(Mode::PushToTalk))],
        ),
        (
            "tap of 200 ms, window closes 500 ms after release",
            &[(0, Down(FN)), (200, Up(FN)), (700, Tick)],
            &[
                (0, Command::Start(Mode::PushToTalk)),
                (700, Command::Cancel(CancelReason::TooShort)),
            ],
        ),
        (
            "held 499 ms is still a tap",
            &[(0, Down(FN)), (499, Up(FN)), (2_000, Tick)],
            &[
                (0, Command::Start(Mode::PushToTalk)),
                (2_000, Command::Cancel(CancelReason::TooShort)),
            ],
        ),
    ];
    check(cases);
}

#[test]
fn next_deadline_tells_the_app_when_to_tick() {
    let mut machine = DictationMachine::new(Bindings::default_for(Platform::MacOs));
    assert_eq!(machine.next_deadline(), None);

    machine.on_key(KeyEvent::press(FN), Duration::from_millis(0));
    machine.on_key(KeyEvent::release(FN), Duration::from_millis(200));

    assert_eq!(machine.next_deadline(), Some(Duration::from_millis(700)));
}

#[test]
fn double_tap_locks_hands_free_and_a_third_quick_tap_cancels() {
    let start = (0, Command::Start(Mode::PushToTalk));
    let cases: &[Case] = &[
        (
            "double-tap locks on the second press and keeps recording",
            &[
                (0, Down(FN)),
                (150, Up(FN)),
                (400, Down(FN)),
                (500, Up(FN)),
                (60_000, Tick),
            ],
            &[start, (400, Command::Lock)],
        ),
        (
            "second press after the window starts a new dictation instead",
            &[(0, Down(FN)), (150, Up(FN)), (650, Down(FN))],
            &[
                start,
                (650, Command::Cancel(CancelReason::TooShort)),
                (650, Command::Start(Mode::PushToTalk)),
            ],
        ),
        (
            "third quick tap cancels",
            &[
                (0, Down(FN)),
                (150, Up(FN)),
                (300, Down(FN)),
                (400, Up(FN)),
                (800, Down(FN)),
                (900, Up(FN)),
            ],
            &[
                start,
                (300, Command::Lock),
                (800, Command::Cancel(CancelReason::TripleTap)),
            ],
        ),
        (
            "a later press while locked stops and inserts",
            &[
                (0, Down(FN)),
                (150, Up(FN)),
                (300, Down(FN)),
                (400, Up(FN)),
                (5_000, Down(FN)),
                (5_100, Up(FN)),
                (9_000, Tick),
            ],
            &[start, (300, Command::Lock), (5_000, Command::StopAndInsert)],
        ),
        (
            "press exactly at the end of the triple-tap window stops",
            &[
                (0, Down(FN)),
                (150, Up(FN)),
                (300, Down(FN)),
                (400, Up(FN)),
                (900, Down(FN)),
            ],
            &[start, (300, Command::Lock), (900, Command::StopAndInsert)],
        ),
    ];
    check(cases);
}

const SPACE: Key = Key::Space;

#[test]
fn hands_free_shortcut_and_ptt_plus_space_escalation() {
    let start = (0, Command::Start(Mode::PushToTalk));
    let cases: &[Case] = &[
        (
            "adding Space while holding push-to-talk locks without restarting",
            &[
                (0, Down(FN)),
                (700, Down(SPACE)),
                (800, Up(SPACE)),
                (900, Up(FN)),
                (60_000, Tick),
            ],
            &[start, (700, Command::Lock)],
        ),
        (
            "Space added inside the tap window still locks rather than discarding",
            &[
                (0, Down(FN)),
                (100, Down(SPACE)),
                (150, Up(SPACE)),
                (200, Up(FN)),
                (2_000, Tick),
            ],
            &[start, (100, Command::Lock)],
        ),
        (
            "the hands-free combo pressed Space-first starts hands-free directly",
            &[
                (0, Down(SPACE)),
                (50, Down(FN)),
                (100, Up(FN)),
                (150, Up(SPACE)),
                (60_000, Tick),
            ],
            &[(50, Command::Start(Mode::HandsFree))],
        ),
        (
            "while locked, push-to-talk stops; finishing the hands-free combo does not restart",
            &[
                (0, Down(FN)),
                (700, Down(SPACE)),
                (800, Up(SPACE)),
                (900, Up(FN)),
                (3_000, Down(FN)),
                (3_050, Down(SPACE)),
                (3_100, Up(SPACE)),
                (3_150, Up(FN)),
                (9_000, Tick),
            ],
            &[start, (700, Command::Lock), (3_000, Command::StopAndInsert)],
        ),
        (
            "while locked by double-tap, the hands-free combo (Space first) stops",
            &[
                (0, Down(FN)),
                (100, Up(FN)),
                (200, Down(FN)),
                (300, Up(FN)),
                (3_000, Down(SPACE)),
                (3_050, Down(FN)),
                (3_100, Up(FN)),
                (3_150, Up(SPACE)),
                (9_000, Tick),
            ],
            &[start, (200, Command::Lock), (3_050, Command::StopAndInsert)],
        ),
        (
            "a new press after everything is released starts again",
            &[
                (0, Down(FN)),
                (700, Down(SPACE)),
                (800, Up(SPACE)),
                (900, Up(FN)),
                (3_000, Down(FN)),
                (3_100, Up(FN)),
                (4_000, Down(FN)),
            ],
            &[
                start,
                (700, Command::Lock),
                (3_000, Command::StopAndInsert),
                (4_000, Command::Start(Mode::PushToTalk)),
            ],
        ),
    ];
    check(cases);
}

const A: Key = Key::Char('A');

#[test]
fn unrelated_key_in_the_first_second_of_push_to_talk_discards() {
    let start = (0, Command::Start(Mode::PushToTalk));
    let stray = |ms| (ms, Command::Cancel(CancelReason::StrayKey));
    let cases: &[Case] = &[
        (
            "letter while holding",
            &[
                (0, Down(FN)),
                (300, Down(A)),
                (350, Up(A)),
                (1_200, Up(FN)),
                (5_000, Tick),
            ],
            &[start, stray(300)],
        ),
        (
            "another modifier while holding",
            &[
                (0, Down(FN)),
                (999, Down(Key::Shift(wf_core::shortcut::Side::Left))),
                (1_500, Up(FN)),
            ],
            &[start, stray(999)],
        ),
        (
            "letter during the double-tap window",
            &[(0, Down(FN)), (100, Up(FN)), (300, Down(A)), (2_000, Tick)],
            &[start, stray(300)],
        ),
        (
            "after the first second other keys are ignored",
            &[
                (0, Down(FN)),
                (1_000, Down(A)),
                (1_100, Up(A)),
                (1_500, Up(FN)),
            ],
            &[start, (1_500, Command::StopAndInsert)],
        ),
        (
            "hands-free (locked) dictation ignores other keys",
            &[
                (0, Down(FN)),
                (100, Up(FN)),
                (200, Down(FN)),
                (300, Up(FN)),
                (600, Down(A)),
                (700, Up(A)),
                (60_000, Tick),
            ],
            &[start, (200, Command::Lock)],
        ),
        (
            "holding the stray key and releasing push-to-talk does not restart",
            &[
                (0, Down(FN)),
                (300, Down(A)),
                (400, Up(FN)),
                (500, Down(FN)),
                (600, Up(FN)),
                (700, Up(A)),
            ],
            &[start, stray(300)],
        ),
    ];
    check(cases);
}

const ESC: Key = Key::Esc;

#[test]
fn cancel_shortcut_works_only_while_dictating() {
    let start = (0, Command::Start(Mode::PushToTalk));
    let cancel = |ms| (ms, Command::Cancel(CancelReason::User));
    let cases: &[Case] = &[
        (
            "idle Esc does nothing",
            &[(0, Down(ESC)), (50, Up(ESC))],
            &[],
        ),
        (
            "while holding push-to-talk",
            &[
                (0, Down(FN)),
                (300, Down(ESC)),
                (350, Up(ESC)),
                (2_000, Up(FN)),
            ],
            &[start, cancel(300)],
        ),
        (
            "during the double-tap window",
            &[
                (0, Down(FN)),
                (100, Up(FN)),
                (200, Down(ESC)),
                (2_000, Tick),
            ],
            &[start, cancel(200)],
        ),
        (
            "while locked",
            &[
                (0, Down(FN)),
                (100, Up(FN)),
                (200, Down(FN)),
                (300, Up(FN)),
                (5_000, Down(ESC)),
                (60_000, Tick),
            ],
            &[start, (200, Command::Lock), cancel(5_000)],
        ),
    ];
    check(cases);
}

#[test]
fn paste_last_fires_only_after_every_key_of_the_combo_is_released() {
    use wf_core::shortcut::Side::Left;
    let (ctrl, cmd, v) = (Key::Ctrl(Left), Key::Meta(Left), Key::Char('V'));
    let cases: &[Case] = &[
        (
            "release in reverse order",
            &[
                (0, Down(ctrl)),
                (50, Down(cmd)),
                (100, Down(v)),
                (150, Up(v)),
                (200, Up(cmd)),
                (250, Up(ctrl)),
            ],
            &[(250, Command::PasteLast)],
        ),
        (
            "release the modifiers first",
            &[
                (0, Down(ctrl)),
                (50, Down(cmd)),
                (100, Down(v)),
                (150, Up(ctrl)),
                (200, Up(cmd)),
                (250, Up(v)),
            ],
            &[(250, Command::PasteLast)],
        ),
        (
            "the modifiers without V do nothing",
            &[
                (0, Down(ctrl)),
                (50, Down(cmd)),
                (150, Up(cmd)),
                (200, Up(ctrl)),
            ],
            &[],
        ),
    ];
    check(cases);
}

#[test]
fn copy_last_and_meeting_notes_fire_on_press() {
    use wf_core::shortcut::Side::{Left, Right};
    let cases: &[Case] = &[
        (
            "copy last (either side of a modifier works)",
            &[
                (0, Down(Key::Ctrl(Right))),
                (50, Down(Key::Meta(Left))),
                (100, Down(Key::Char('C'))),
                (150, Up(Key::Char('C'))),
            ],
            &[(100, Command::CopyLast)],
        ),
        (
            "meeting notes",
            &[
                (0, Down(Key::Alt(Left))),
                (50, Down(Key::Char('M'))),
                (100, Up(Key::Char('M'))),
                (150, Up(Key::Alt(Left))),
            ],
            &[(50, Command::ToggleMeetingNotes)],
        ),
    ];
    check(cases);
}

const MIN: u64 = 60_000;

#[test]
fn dictation_is_capped_at_20_minutes_with_a_warning_at_19() {
    let cases: &[Case] = &[
        (
            "push-to-talk held past the cap",
            &[
                (0, Down(FN)),
                (19 * MIN - 1, Tick),
                (19 * MIN, Tick),
                (20 * MIN - 1, Tick),
                (20 * MIN, Tick),
                (20 * MIN + 500, Up(FN)),
            ],
            &[
                (0, Command::Start(Mode::PushToTalk)),
                (19 * MIN, Command::CapWarning),
                (20 * MIN, Command::StopAndInsert),
            ],
        ),
        (
            "locked dictation, counted from the first press",
            &[
                (1_000, Down(FN)),
                (1_100, Up(FN)),
                (1_200, Down(FN)),
                (1_300, Up(FN)),
                (19 * MIN + 1_000, Tick),
                (20 * MIN + 1_000, Tick),
            ],
            &[
                (1_000, Command::Start(Mode::PushToTalk)),
                (1_200, Command::Lock),
                (19 * MIN + 1_000, Command::CapWarning),
                (20 * MIN + 1_000, Command::StopAndInsert),
            ],
        ),
        (
            "a late tick applies both",
            &[(0, Down(SPACE)), (0, Down(FN)), (25 * MIN, Tick)],
            &[
                (0, Command::Start(Mode::HandsFree)),
                (25 * MIN, Command::CapWarning),
                (25 * MIN, Command::StopAndInsert),
            ],
        ),
    ];
    check(cases);
}

#[test]
fn next_deadline_covers_the_cap() {
    let mut machine = DictationMachine::new(Bindings::default_for(Platform::MacOs));
    machine.on_key(KeyEvent::press(FN), Duration::ZERO);
    assert_eq!(machine.next_deadline(), Some(Duration::from_secs(19 * 60)));

    machine.tick(Duration::from_secs(19 * 60));
    assert_eq!(machine.next_deadline(), Some(Duration::from_secs(20 * 60)));

    machine.tick(Duration::from_secs(20 * 60));
    assert_eq!(machine.next_deadline(), None);
}

/// Runs `script` (no ticks) and returns, per event, whether the backend should swallow it.
fn swallowed(script: &[(u64, Step)]) -> Vec<bool> {
    let mut machine = DictationMachine::new(Bindings::default_for(Platform::MacOs));
    script
        .iter()
        .map(|&(ms, step)| {
            let now = Duration::from_millis(ms);
            match step {
                Down(key) => machine.on_key(KeyEvent::press(key), now).swallow,
                Up(key) => machine.on_key(KeyEvent::release(key), now).swallow,
                Tick => panic!("no ticks here"),
            }
        })
        .collect()
}

#[test]
fn keys_consumed_by_a_shortcut_are_swallowed_but_modifiers_and_typing_pass_through() {
    use wf_core::shortcut::Side::Left;
    let cases: &[(&str, Script, &[bool])] = &[
        (
            "Space that locks, and its release; Fn passes",
            &[
                (0, Down(FN)),
                (700, Down(SPACE)),
                (800, Up(SPACE)),
                (900, Up(FN)),
            ],
            &[false, true, true, false],
        ),
        (
            "Esc while dictating, not when idle",
            &[
                (0, Down(FN)),
                (300, Down(ESC)),
                (350, Up(ESC)),
                (400, Up(FN)),
                (600, Down(ESC)),
                (650, Up(ESC)),
            ],
            &[false, true, true, false, false, false],
        ),
        (
            "the V of paste-last; the stray key that discards passes",
            &[
                (0, Down(Key::Ctrl(Left))),
                (50, Down(Key::Meta(Left))),
                (100, Down(Key::Char('V'))),
                (150, Up(Key::Char('V'))),
                (200, Up(Key::Meta(Left))),
                (250, Up(Key::Ctrl(Left))),
                (1_000, Down(FN)),
                (1_200, Down(A)),
                (1_300, Up(A)),
            ],
            &[false, false, true, true, false, false, false, false, false],
        ),
        (
            "Space finishing the hands-free combo whose Fn already stopped (a leading Space can't be)",
            &[
                (0, Down(SPACE)),
                (50, Down(FN)),
                (100, Up(FN)),
                (150, Up(SPACE)),
                (3_000, Down(FN)),
                (3_050, Down(SPACE)),
                (3_100, Up(SPACE)),
            ],
            &[false, false, false, false, false, true, true],
        ),
        (
            "plain typing",
            &[(0, Down(A)), (50, Up(A))],
            &[false, false],
        ),
    ];
    for (name, script, expected) in cases {
        assert_eq!(swallowed(script), *expected, "{name}");
    }
}

#[test]
fn pill_click_toggles_hands_free_and_its_buttons_stop_or_cancel() {
    use wf_core::dictation::PillInput::{Cancel, Click, Stop};
    let mut machine = DictationMachine::new(Bindings::default_for(Platform::MacOs));
    let at = Duration::from_millis;

    assert_eq!(machine.on_pill(Stop, at(0)), []);
    assert_eq!(
        machine.on_pill(Click, at(0)),
        [Command::Start(Mode::HandsFree)]
    );
    assert_eq!(machine.on_pill(Click, at(5_000)), [Command::StopAndInsert]);
    assert_eq!(
        machine.on_pill(Click, at(6_000)),
        [Command::Start(Mode::HandsFree)]
    );
    assert_eq!(
        machine.on_pill(Cancel, at(7_000)),
        [Command::Cancel(CancelReason::User)]
    );
    assert_eq!(
        machine.on_pill(Click, at(8_000)),
        [Command::Start(Mode::HandsFree)]
    );
    assert_eq!(machine.on_pill(Stop, at(9_000)), [Command::StopAndInsert]);

    // Shortcuts still work right after a pill action.
    let reaction = machine.on_key(KeyEvent::press(FN), at(10_000));
    assert_eq!(reaction.commands, [Command::Start(Mode::PushToTalk)]);
}

#[test]
fn multi_key_push_to_talk_on_windows() {
    use wf_core::shortcut::Side::{Left, Right};
    let mut machine = DictationMachine::new(Bindings::default_for(Platform::Windows));
    let mut feed = |event, ms| machine.on_key(event, Duration::from_millis(ms)).commands;

    assert_eq!(feed(KeyEvent::press(Key::Meta(Left)), 0), []);
    assert_eq!(
        feed(KeyEvent::press(Key::Ctrl(Right)), 40),
        [Command::Start(Mode::PushToTalk)]
    );
    assert_eq!(feed(KeyEvent::press(Key::Space), 900), [Command::Lock]);
    assert_eq!(feed(KeyEvent::release(Key::Space), 950), []);
    assert_eq!(feed(KeyEvent::release(Key::Ctrl(Right)), 1_000), []);
    assert_eq!(feed(KeyEvent::release(Key::Meta(Left)), 1_050), []);
    assert_eq!(feed(KeyEvent::press(Key::Ctrl(Left)), 4_000), []);
    assert_eq!(
        feed(KeyEvent::press(Key::Meta(Left)), 4_050),
        [Command::StopAndInsert]
    );
}

#[test]
fn auto_repeat_of_a_swallowed_key_stays_swallowed() {
    let swallow = swallowed(&[
        (0, Down(FN)),
        (700, Down(SPACE)),
        (1_200, Down(SPACE)),
        (1_230, Down(SPACE)),
        (1_300, Up(SPACE)),
    ]);
    assert_eq!(swallow, [false, true, true, true, true]);
}

#[test]
fn push_to_talk_ends_only_when_its_combo_is_no_longer_held() {
    use wf_core::shortcut::Side::{Left, Right};
    let mut machine = DictationMachine::new(Bindings::default_for(Platform::Windows));
    let mut feed = |event, ms| machine.on_key(event, Duration::from_millis(ms)).commands;

    feed(KeyEvent::press(Key::Ctrl(Left)), 0);
    assert_eq!(
        feed(KeyEvent::press(Key::Meta(Left)), 10),
        [Command::Start(Mode::PushToTalk)]
    );
    feed(KeyEvent::press(Key::Ctrl(Right)), 700);
    assert_eq!(feed(KeyEvent::release(Key::Ctrl(Right)), 800), []);
    assert_eq!(
        feed(KeyEvent::release(Key::Ctrl(Left)), 900),
        [Command::StopAndInsert]
    );
}

#[test]
fn time_running_backwards_never_panics() {
    let script = [
        (100, Down(FN)),
        (50, Up(FN)),
        (20, Down(FN)),
        (10, Down(A)),
        (5, Up(FN)),
        (0, Tick),
    ];
    run(&script);
}

#[test]
fn losing_the_key_stream_forgets_held_keys_and_ends_push_to_talk() {
    let at = Duration::from_millis;
    let mut machine = DictationMachine::new(Bindings::default_for(Platform::MacOs));

    // Fn is held and its release will never arrive.
    machine.on_key(KeyEvent::press(FN), at(0));
    assert_eq!(machine.on_keys_lost(at(800)), [Command::StopAndInsert]);

    // A key stuck "down" no longer blocks the exact match, and the phantom key can't double-fire.
    machine.on_key(KeyEvent::press(A), at(1_000));
    machine.on_keys_lost(at(1_100));
    assert_eq!(
        machine.on_key(KeyEvent::press(FN), at(2_000)).commands,
        [Command::Start(Mode::PushToTalk)]
    );
    assert_eq!(machine.on_key(KeyEvent::release(A), at(2_100)).commands, []);

    // A locked dictation doesn't depend on held keys, so it carries on.
    machine.on_key(KeyEvent::press(SPACE), at(2_200));
    assert_eq!(machine.on_keys_lost(at(2_300)), []);
    assert_eq!(
        machine.on_pill(wf_core::dictation::PillInput::Stop, at(3_000)),
        [Command::StopAndInsert]
    );
}

fn machine() -> DictationMachine {
    DictationMachine::new(Bindings::default_for(Platform::MacOs))
}

fn at(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

#[test]
fn esc_cancels_processing_and_is_kept_from_the_focused_app() {
    let mut machine = machine();
    machine.on_key(KeyEvent::press(FN), at(0));
    machine.on_key(KeyEvent::release(FN), at(800));
    machine.set_processing(true);

    let reaction = machine.on_key(KeyEvent::press(Key::Esc), at(900));

    assert_eq!(reaction.commands, [Command::Cancel(CancelReason::User)]);
    assert!(reaction.swallow);
    assert!(machine.on_key(KeyEvent::release(Key::Esc), at(950)).swallow);
}

#[test]
fn esc_passes_through_once_processing_is_over() {
    let mut machine = machine();
    machine.set_processing(true);
    machine.set_processing(false);

    let reaction = machine.on_key(KeyEvent::press(Key::Esc), at(0));

    assert_eq!(reaction, Default::default());
}

#[test]
fn a_new_dictation_can_start_while_the_last_one_is_processing() {
    let mut machine = machine();
    machine.set_processing(true);

    let reaction = machine.on_key(KeyEvent::press(FN), at(0));

    assert_eq!(reaction.commands, [Command::Start(Mode::PushToTalk)]);
}

#[test]
fn new_bindings_apply_to_the_next_press() {
    let mut machine = machine();
    let mut bindings = Bindings::default_for(Platform::MacOs);
    bindings.push_to_talk = Combo::new([Key::F(13)]);

    machine.set_bindings(bindings);

    assert_eq!(machine.on_key(KeyEvent::press(FN), at(0)).commands, []);
    machine.on_key(KeyEvent::release(FN), at(10));
    assert_eq!(
        machine.on_key(KeyEvent::press(Key::F(13)), at(20)).commands,
        [Command::Start(Mode::PushToTalk)]
    );
}

#[test]
fn abandon_ends_a_dictation_without_a_command() {
    let mut machine = machine();
    machine.on_key(KeyEvent::press(FN), at(0));

    machine.abandon();

    assert_eq!(machine.next_deadline(), None);
    assert_eq!(machine.on_key(KeyEvent::release(FN), at(800)).commands, []);
}
