use wf_core::pill::dock::{
    Display, Edge, Point, Rect, Size, display_at, dock_origin, nearest_edge,
};

fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

/// A 1000x800 laptop screen with a 25-high menu bar and a 75-high Dock.
fn laptop() -> Display {
    Display {
        bounds: rect(0.0, 0.0, 1000.0, 800.0),
        work_area: rect(0.0, 25.0, 1000.0, 700.0),
        units_per_point: 1.0,
    }
}

/// A larger monitor to the right, offset 100 down from the laptop's top.
fn external() -> Display {
    Display {
        bounds: rect(1000.0, 100.0, 2000.0, 1200.0),
        work_area: rect(1000.0, 100.0, 2000.0, 1200.0),
        units_per_point: 1.0,
    }
}

const WINDOW: Size = Size {
    width: 300.0,
    height: 100.0,
};

#[test]
fn display_at_picks_the_screen_under_the_cursor() {
    let displays = [laptop(), external()];
    let on_external = display_at(
        &displays,
        Point {
            x: 1500.0,
            y: 500.0,
        },
    );
    assert_eq!(on_external, Some(&displays[1]));
    let on_laptop = display_at(&displays, Point { x: 10.0, y: 10.0 });
    assert_eq!(on_laptop, Some(&displays[0]));
}

#[test]
fn display_at_falls_back_to_the_nearest_screen_when_the_cursor_is_in_a_gap() {
    let displays = [laptop(), external()];
    // Below the laptop's bottom edge but left of the monitor.
    let picked = display_at(&displays, Point { x: 900.0, y: 830.0 });
    assert_eq!(picked, Some(&displays[0]));
    assert_eq!(display_at(&[], Point { x: 0.0, y: 0.0 }), None);
}

#[test]
fn bottom_dock_is_centred_and_sits_on_the_top_of_the_dock() {
    let origin = dock_origin(&laptop(), Edge::Bottom, WINDOW);
    assert_eq!(origin, Point { x: 350.0, y: 625.0 });
}

#[test]
fn side_docks_are_vertically_centred_in_the_work_area() {
    assert_eq!(
        dock_origin(&laptop(), Edge::Left, WINDOW),
        Point { x: 0.0, y: 325.0 }
    );
    assert_eq!(
        dock_origin(&laptop(), Edge::Right, WINDOW),
        Point { x: 700.0, y: 325.0 }
    );
}

#[test]
fn docking_uses_the_work_area_of_a_display_with_an_offset_origin() {
    let origin = dock_origin(&external(), Edge::Right, WINDOW);
    assert_eq!(
        origin,
        Point {
            x: 2700.0,
            y: 650.0
        }
    );
}

#[test]
fn window_size_is_scaled_to_desktop_units_on_scaled_displays() {
    let hidpi = Display {
        bounds: rect(0.0, 0.0, 2000.0, 1000.0),
        work_area: rect(0.0, 0.0, 2000.0, 960.0),
        units_per_point: 2.0,
    };
    let origin = dock_origin(&hidpi, Edge::Bottom, WINDOW);
    assert_eq!(origin, Point { x: 700.0, y: 760.0 });
}

#[test]
fn nearest_edge_is_bottom_left_or_right_never_top() {
    let display = laptop();
    let at = |x, y| nearest_edge(&display, Point { x, y });
    assert_eq!(at(500.0, 700.0), Edge::Bottom);
    assert_eq!(at(20.0, 400.0), Edge::Left);
    assert_eq!(at(980.0, 400.0), Edge::Right);
    // Near the top the pill still goes to the closest allowed edge.
    assert_eq!(at(100.0, 30.0), Edge::Left);
    assert_eq!(at(900.0, 30.0), Edge::Right);
}

#[test]
fn edge_round_trips_through_its_saved_name() {
    for edge in [Edge::Bottom, Edge::Left, Edge::Right] {
        assert_eq!(edge.as_str().parse(), Ok(edge));
    }
    assert!("top".parse::<Edge>().is_err());
}
