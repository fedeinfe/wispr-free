use std::time::Duration;

use wf_core::pill::dock::{Display, Edge, Point, Rect};
use wf_core::pill::{Frame, Pill, PillCommand, PillState, Sound, Tick, WINDOW_SIZE};

fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn laptop() -> Display {
    Display {
        bounds: rect(0.0, 0.0, 1000.0, 800.0),
        work_area: rect(0.0, 25.0, 1000.0, 700.0),
        units_per_point: 1.0,
    }
}

fn external() -> Display {
    Display {
        bounds: rect(1000.0, 0.0, 2000.0, 1200.0),
        work_area: rect(1000.0, 0.0, 2000.0, 1200.0),
        units_per_point: 1.0,
    }
}

/// Plays a pill's inputs one poll at a time, like the app's timer does.
struct Harness {
    pill: Pill,
    displays: Vec<Display>,
    now: Duration,
}

impl Harness {
    fn new(edge: Edge) -> Self {
        let mut harness = Self {
            pill: Pill::new(edge),
            displays: vec![laptop()],
            now: Duration::ZERO,
        };
        harness.cursor(5.0, 5.0);
        harness
    }

    fn tick(&mut self, cursor: Option<Point>, escape: bool) -> Frame {
        self.now += Duration::from_millis(16);
        self.pill.tick(&Tick {
            now: self.now,
            cursor,
            displays: &self.displays,
            escape,
        })
    }

    fn cursor(&mut self, x: f64, y: f64) -> Frame {
        self.tick(Some(Point { x, y }), false)
    }

    /// A point on the capsule, in desktop coordinates, for the current dock.
    fn on_capsule(&mut self) -> Point {
        let frame = self.cursor(5.0, 5.0);
        let origin = self.pill.origin();
        Point {
            x: origin.x + frame.capsule.x + frame.capsule.width / 2.0,
            y: origin.y + frame.capsule.y + frame.capsule.height / 2.0,
        }
    }

    fn hover(&mut self) -> Frame {
        let p = self.on_capsule();
        self.cursor(p.x, p.y)
    }

    fn away(&mut self) -> Frame {
        self.cursor(5.0, 5.0)
    }
}

fn error() -> PillState {
    PillState::Error {
        reason: "No microphone found".into(),
    }
}

#[test]
fn hovering_a_resting_pill_shows_ready_and_leaving_returns_to_resting() {
    let mut h = Harness::new(Edge::Bottom);
    assert_eq!(h.away().shown, PillState::Resting);
    assert_eq!(h.hover().shown, PillState::Ready);
    assert_eq!(h.away().shown, PillState::Resting);
}

#[test]
fn hovering_does_not_change_the_look_of_an_active_state() {
    let mut h = Harness::new(Edge::Bottom);
    h.pill.set_state(PillState::Processing);
    assert_eq!(h.hover().shown, PillState::Processing);
}

#[test]
fn clicks_pass_through_except_over_the_capsule() {
    let mut h = Harness::new(Edge::Bottom);
    assert!(!h.away().hit_test);
    assert!(h.hover().hit_test);
}

#[test]
fn just_outside_the_capsule_it_hovers_but_clicks_still_pass_through() {
    let mut h = Harness::new(Edge::Bottom);
    let frame = h.hover();
    let origin = h.pill.origin();
    let just_above = (
        origin.x + frame.capsule.x + 10.0,
        origin.y + frame.capsule.y - 3.0,
    );
    let near = h.cursor(just_above.0, just_above.1);
    assert_eq!(near.shown, PillState::Ready);
    assert!(!near.hit_test);
}

#[test]
fn a_hidden_pill_catches_nothing_even_with_the_cursor_on_its_spot() {
    let mut h = Harness::new(Edge::Bottom);
    let p = h.on_capsule();
    h.pill.set_state(PillState::Hidden);
    assert_eq!(h.away().shown, PillState::Hidden);
    let over = h.cursor(p.x, p.y);
    assert_eq!(over.shown, PillState::Hidden);
    assert!(!over.hit_test);
}

#[test]
fn without_a_global_cursor_the_pill_stays_clickable_and_on_the_first_display() {
    let mut h = Harness::new(Edge::Bottom);
    h.displays = vec![external(), laptop()];
    let frame = h.tick(None, false);
    assert!(frame.hit_test);
    assert_eq!(frame.shown, PillState::Resting);
    assert_eq!(
        frame.move_to.map(|p| p.x),
        Some(1000.0 + 1000.0 - WINDOW_SIZE.width / 2.0)
    );
}

#[test]
fn the_first_tick_docks_the_window_then_it_stays_put() {
    let mut pill = Pill::new(Edge::Bottom);
    let tick = Tick {
        now: Duration::ZERO,
        cursor: Some(Point { x: 5.0, y: 5.0 }),
        displays: &[laptop()],
        escape: false,
    };
    let first = pill.tick(&tick);
    let expected = Point {
        x: 500.0 - WINDOW_SIZE.width / 2.0,
        y: 725.0 - WINDOW_SIZE.height,
    };
    assert_eq!(first.move_to, Some(expected));
    assert_eq!(pill.tick(&tick).move_to, None);
}

#[test]
fn the_pill_follows_the_display_the_cursor_is_on() {
    let mut h = Harness::new(Edge::Bottom);
    h.displays.push(external());
    let moved = h.cursor(1500.0, 300.0).move_to.expect("should move");
    assert_eq!(moved.y, 1200.0 - WINDOW_SIZE.height);
    assert_eq!(moved.x, 2000.0 - WINDOW_SIZE.width / 2.0);
    assert_eq!(h.cursor(1600.0, 300.0).move_to, None);
    assert!(h.cursor(10.0, 10.0).move_to.is_some());
}

#[test]
fn the_pill_redocks_when_the_dock_or_taskbar_changes_size() {
    let mut h = Harness::new(Edge::Bottom);
    h.displays[0].work_area.height = 600.0;
    let moved = h.away().move_to.expect("should move");
    assert_eq!(moved.y, 625.0 - WINDOW_SIZE.height);
}

#[test]
fn the_pill_keeps_its_place_when_no_displays_are_reported() {
    let mut h = Harness::new(Edge::Bottom);
    let before = h.pill.origin();
    h.displays.clear();
    assert_eq!(h.away().move_to, None);
    assert_eq!(h.pill.origin(), before);
}

#[test]
fn clicking_ready_toggles_hands_free() {
    let mut h = Harness::new(Edge::Bottom);
    h.hover();
    h.pill.press();
    assert_eq!(h.pill.release(), Some(PillCommand::ToggleHandsFree));
}

#[test]
fn a_small_tremble_during_the_click_is_still_a_click() {
    let mut h = Harness::new(Edge::Bottom);
    let p = h.on_capsule();
    h.cursor(p.x, p.y);
    h.pill.press();
    let frame = h.cursor(p.x + 2.0, p.y - 1.0);
    assert_eq!(frame.move_to, None);
    assert_eq!(h.pill.release(), Some(PillCommand::ToggleHandsFree));
}

#[test]
fn clicking_outside_ready_does_nothing() {
    for state in [
        PillState::ListeningPtt,
        PillState::ListeningHandsFree,
        PillState::Processing,
        PillState::Meeting,
        error(),
    ] {
        let mut h = Harness::new(Edge::Bottom);
        h.pill.set_state(state);
        h.hover();
        h.pill.press();
        assert_eq!(h.pill.release(), None);
    }
}

#[test]
fn dragging_moves_the_window_with_the_cursor() {
    let mut h = Harness::new(Edge::Bottom);
    let p = h.on_capsule();
    h.cursor(p.x, p.y);
    let start = h.pill.origin();
    h.pill.press();
    let frame = h.cursor(p.x - 100.0, p.y - 50.0);
    assert_eq!(
        frame.move_to,
        Some(Point {
            x: start.x - 100.0,
            y: start.y - 50.0
        })
    );
    assert!(
        frame.hit_test,
        "the capsule must keep the mouse while dragged"
    );
}

#[test]
fn dropping_near_an_edge_snaps_and_saves_that_edge() {
    let mut h = Harness::new(Edge::Bottom);
    let p = h.on_capsule();
    h.cursor(p.x, p.y);
    h.pill.press();
    h.cursor(30.0, 400.0);
    assert_eq!(h.pill.release(), Some(PillCommand::EdgeChanged(Edge::Left)));
    assert_eq!(h.pill.edge(), Edge::Left);
    let docked = h.cursor(30.0, 400.0).move_to.expect("should snap");
    assert_eq!(docked.x, 0.0);
    assert_eq!(docked.y, 25.0 + (700.0 - WINDOW_SIZE.height) / 2.0);
}

#[test]
fn dropping_back_on_the_same_edge_snaps_without_saving() {
    let mut h = Harness::new(Edge::Bottom);
    let home = h.on_capsule();
    h.cursor(home.x, home.y);
    let origin = h.pill.origin();
    h.pill.press();
    h.cursor(home.x + 40.0, home.y - 30.0);
    assert_eq!(h.pill.release(), None);
    assert_eq!(h.cursor(home.x + 40.0, home.y - 30.0).move_to, Some(origin));
}

#[test]
fn escape_cancels_a_drag_and_the_release_is_ignored() {
    let mut h = Harness::new(Edge::Bottom);
    let p = h.on_capsule();
    h.cursor(p.x, p.y);
    let origin = h.pill.origin();
    h.pill.press();
    h.cursor(30.0, 400.0);
    let cancelled = h.tick(Some(Point { x: 30.0, y: 400.0 }), true);
    assert_eq!(cancelled.move_to, Some(origin));
    assert_eq!(h.cursor(25.0, 400.0).move_to, None);
    assert_eq!(h.pill.release(), None);
    assert_eq!(h.pill.edge(), Edge::Bottom);
}

#[test]
fn a_held_escape_does_not_affect_an_ordinary_click() {
    let mut h = Harness::new(Edge::Bottom);
    let p = h.on_capsule();
    h.tick(Some(p), true);
    h.pill.press();
    h.tick(Some(p), true);
    assert_eq!(h.pill.release(), Some(PillCommand::ToggleHandsFree));
}

#[test]
fn a_drop_docks_on_the_display_under_the_cursor() {
    let mut h = Harness::new(Edge::Bottom);
    h.displays.push(external());
    let p = h.on_capsule();
    h.cursor(p.x, p.y);
    h.pill.press();
    h.cursor(1500.0, 300.0);
    // Near no edge of the monitor in particular: the bottom is 900 away, the left 500.
    assert_eq!(h.pill.release(), Some(PillCommand::EdgeChanged(Edge::Left)));
    let docked = h.cursor(1500.0, 300.0).move_to.expect("should snap");
    assert_eq!(
        docked.x, 1000.0,
        "should dock on the monitor it was dropped on"
    );
}

#[test]
fn hide_for_hides_until_the_time_is_up_then_restores_the_state() {
    let mut h = Harness::new(Edge::Bottom);
    h.away();
    h.pill.hide_for(Duration::from_secs(3600));
    assert_eq!(h.away().shown, PillState::Hidden);

    h.now += Duration::from_secs(3599);
    assert_eq!(h.away().shown, PillState::Hidden);

    h.now += Duration::from_secs(2);
    assert_eq!(h.away().shown, PillState::Resting);
}

#[test]
fn a_dictation_during_hide_for_stays_hidden_but_keeps_its_state() {
    let mut h = Harness::new(Edge::Bottom);
    h.away();
    h.pill.hide_for(Duration::from_secs(3600));
    h.pill.set_state(PillState::ListeningPtt);
    assert_eq!(h.away().shown, PillState::Hidden);
    assert_eq!(h.pill.state(), &PillState::ListeningPtt);
}

#[test]
fn with_the_pill_turned_off_a_dictation_stays_hidden_and_keeps_its_cues() {
    let mut h = Harness::new(Edge::Bottom);
    h.away();
    h.pill.set_turned_off(true);

    assert_eq!(
        h.pill.set_state(PillState::ListeningPtt),
        Some(Sound::Start)
    );
    assert_eq!(h.away().shown, PillState::Hidden);
    assert_eq!(h.pill.state(), &PillState::ListeningPtt);

    h.pill.set_turned_off(false);
    assert_eq!(h.away().shown, PillState::ListeningPtt);
}

#[test]
fn with_the_pill_turned_off_an_error_still_shows() {
    let mut h = Harness::new(Edge::Bottom);
    h.away();
    h.pill.set_turned_off(true);
    let error = PillState::Error {
        reason: "No microphone found".into(),
    };

    h.pill.set_state(error.clone());

    assert_eq!(h.away().shown, error);
}

#[test]
fn the_suggestion_card_fits_the_window_on_every_edge_and_clears_the_capsule() {
    for edge in [Edge::Bottom, Edge::Left, Edge::Right] {
        let mut h = Harness::new(edge);
        h.pill.set_state(PillState::Meeting);
        h.pill.set_card(true);

        let frame = h.away();

        let card = frame.card.expect("the card is up");
        assert!(card.x >= 0.0 && card.y >= 0.0, "{edge:?}");
        assert!(card.x + card.width <= WINDOW_SIZE.width, "{edge:?}");
        assert!(card.y + card.height <= WINDOW_SIZE.height, "{edge:?}");
        let capsule = frame.capsule;
        let apart = card.x + card.width <= capsule.x
            || capsule.x + capsule.width <= card.x
            || card.y + card.height <= capsule.y
            || capsule.y + capsule.height <= card.y;
        assert!(apart, "{edge:?}");
    }
}

#[test]
fn the_card_catches_the_mouse_only_while_it_is_up() {
    let mut h = Harness::new(Edge::Bottom);
    h.away();
    h.pill.set_card(true);
    let card = h.away().card.unwrap();
    let origin = h.pill.origin();
    let over = (origin.x + card.x + 5.0, origin.y + card.y + 5.0);

    assert!(h.cursor(over.0, over.1).hit_test);

    h.pill.set_card(false);
    assert!(!h.cursor(over.0, over.1).hit_test);
}

#[test]
fn the_card_shows_with_the_pill_turned_off() {
    let mut h = Harness::new(Edge::Bottom);
    h.pill.set_turned_off(true);
    h.pill.set_card(true);

    assert!(h.away().card.is_some());
}

#[test]
fn capsules_grow_from_resting_to_hands_free() {
    let width = |state: PillState| state.capsule_size(Edge::Bottom).width;
    assert!(width(PillState::Resting) < width(PillState::Ready));
    assert!(width(PillState::Ready) < width(PillState::ListeningPtt));
    assert!(width(PillState::ListeningPtt) < width(PillState::ListeningHandsFree));
}

#[test]
fn the_resting_capsule_lies_along_a_side_edge() {
    let along = PillState::Resting.capsule_size(Edge::Bottom);
    let side = PillState::Resting.capsule_size(Edge::Left);
    assert_eq!((side.width, side.height), (along.height, along.width));
}

#[test]
fn the_capsule_sits_against_the_docked_edge_of_the_window() {
    let capsule = |edge| {
        let mut h = Harness::new(edge);
        h.pill.set_state(PillState::ListeningPtt);
        h.away().capsule
    };
    let bottom = capsule(Edge::Bottom);
    assert_eq!(bottom.x + bottom.width / 2.0, WINDOW_SIZE.width / 2.0);
    assert!(bottom.y + bottom.height > WINDOW_SIZE.height - 16.0);

    let left = capsule(Edge::Left);
    assert!(left.x < 16.0);
    assert_eq!(left.y + left.height / 2.0, WINDOW_SIZE.height / 2.0);

    let right = capsule(Edge::Right);
    assert!(right.x + right.width > WINDOW_SIZE.width - 16.0);
}

#[test]
fn every_capsule_leaves_room_in_the_window_for_a_tooltip() {
    let frame_room = |state: PillState, edge| {
        let mut h = Harness::new(edge);
        h.pill.set_state(state);
        h.away().capsule.y
    };
    for edge in [Edge::Bottom, Edge::Left, Edge::Right] {
        assert!(frame_room(error(), edge) >= 30.0);
        assert!(frame_room(PillState::ListeningHandsFree, edge) >= 30.0);
    }
}

#[test]
fn only_the_error_state_has_a_tooltip() {
    assert_eq!(error().tooltip(), Some("No microphone found"));
    assert_eq!(PillState::Processing.tooltip(), None);
}

#[test]
fn only_listening_and_meeting_show_bars_and_meeting_bars_are_shorter() {
    let scale = |state: PillState| state.bar_scale();
    assert_eq!(scale(PillState::ListeningPtt), Some(1.0));
    assert_eq!(scale(PillState::ListeningHandsFree), Some(1.0));
    let meeting = scale(PillState::Meeting).expect("meeting shows bars");
    assert!(meeting < 1.0);
    assert_eq!(scale(PillState::Ready), None);
    assert_eq!(scale(PillState::Processing), None);
}

#[test]
fn state_changes_cue_the_right_sound() {
    use PillState::*;
    let cue = |from: PillState, to: PillState| {
        let mut pill = Pill::new(Edge::Bottom);
        pill.set_state(from);
        pill.set_state(to)
    };
    assert_eq!(cue(Resting, ListeningPtt), Some(Sound::Start));
    assert_eq!(cue(Ready, ListeningHandsFree), Some(Sound::Start));
    assert_eq!(cue(ListeningPtt, ListeningHandsFree), Some(Sound::Lock));
    assert_eq!(cue(ListeningPtt, Processing), Some(Sound::Stop));
    assert_eq!(cue(ListeningHandsFree, Resting), Some(Sound::Stop));
    assert_eq!(cue(Processing, error()), Some(Sound::Error));
    assert_eq!(cue(Processing, Resting), None);
    assert_eq!(cue(Resting, Ready), None);
    assert_eq!(cue(error(), error()), None);
}

#[test]
fn the_pill_starts_on_the_saved_edge() {
    assert_eq!(Pill::new(Edge::Right).edge(), Edge::Right);
    assert_eq!(Pill::new(Edge::Right).state(), &PillState::Resting);
}
