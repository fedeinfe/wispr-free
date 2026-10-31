//! The rules for opening and closing a window, with a fake window that only keeps count.

#[path = "support/clock.rs"]
mod clock;

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use wf_shell::windows::{Window, WindowHost};

use clock::ManualClock;

#[derive(Default)]
struct Counts {
    made: Cell<u32>,
    shown: Cell<u32>,
    hidden: Cell<u32>,
    brought_to_front: Cell<u32>,
    started: Cell<u32>,
}

struct FakeWindow {
    counts: Rc<Counts>,
    close: RefCell<Option<Box<dyn Fn()>>>,
    alive: Rc<()>,
}

impl Window for FakeWindow {
    fn show(&self) -> Result<(), String> {
        self.counts.shown.set(self.counts.shown.get() + 1);
        Ok(())
    }

    fn hide(&self) {
        self.counts.hidden.set(self.counts.hidden.get() + 1);
    }

    fn bring_to_front(&self) {
        let count = &self.counts.brought_to_front;
        count.set(count.get() + 1);
    }

    fn on_close(&self, close: Box<dyn Fn()>) {
        *self.close.borrow_mut() = Some(close);
    }

    fn start(&self) {
        self.counts.started.set(self.counts.started.get() + 1);
    }
}

struct Harness {
    counts: Rc<Counts>,
    clock: Rc<ManualClock>,
    host: WindowHost<FakeWindow>,
}

impl Harness {
    fn new() -> Self {
        let counts = Rc::new(Counts::default());
        let clock = Rc::new(ManualClock::default());
        let make = {
            let counts = Rc::clone(&counts);
            Rc::new(move || {
                counts.made.set(counts.made.get() + 1);
                Ok(FakeWindow {
                    counts: Rc::clone(&counts),
                    close: RefCell::default(),
                    alive: Rc::new(()),
                })
            })
        };
        let host = WindowHost::new(make, clock.clone());
        Self {
            counts,
            clock,
            host,
        }
    }

    fn alive(&self) -> Weak<()> {
        self.host
            .with_window(|window| Rc::downgrade(&window.alive))
            .unwrap()
    }

    /// As the user closes the window, which only asks for a close once the callback has returned.
    fn close_button(&self) {
        self.host.with_window(|window| {
            (window.close.borrow().as_ref().unwrap())();
        });
    }
}

#[test]
fn the_window_exists_only_while_open() {
    let harness = Harness::new();
    assert!(!harness.host.is_open());

    harness.host.open().unwrap();

    assert!(harness.host.is_open());
    assert_eq!(harness.counts.brought_to_front.get(), 1);
    assert_eq!(harness.counts.started.get(), 1);
}

#[test]
fn opening_twice_brings_the_same_window_forward() {
    let harness = Harness::new();
    harness.host.open().unwrap();
    let first = harness.alive();

    harness.host.open().unwrap();

    assert_eq!(harness.counts.brought_to_front.get(), 2);
    assert_eq!(harness.counts.made.get(), 1, "no second window");
    assert_eq!(harness.counts.started.get(), 1);
    assert_eq!(harness.counts.shown.get(), 2);
    assert!(first.upgrade().is_some());
}

#[test]
fn closing_the_window_drops_it() {
    let harness = Harness::new();
    harness.host.open().unwrap();
    let window = harness.alive();

    harness.close_button();
    assert!(harness.host.is_open(), "not from inside its own callback");
    harness.clock.advance(Duration::from_millis(1));

    assert!(!harness.host.is_open());
    assert!(window.upgrade().is_none());
    assert_eq!(harness.counts.hidden.get(), 1);
}

#[test]
fn the_window_can_be_opened_again_after_closing() {
    let harness = Harness::new();
    harness.host.open().unwrap();
    harness.host.close();

    harness.host.open().unwrap();

    assert!(harness.host.is_open());
    assert_eq!(harness.counts.made.get(), 2);
}

#[test]
fn a_host_that_goes_away_leaves_a_close_with_nothing_to_do() {
    let harness = Harness::new();
    harness.host.open().unwrap();
    let close = harness
        .host
        .with_window(|window| window.close.take())
        .unwrap();
    let Harness { host, clock, .. } = harness;
    let weak = host.with_window(|window| Rc::downgrade(&window.alive));

    drop(host);
    close.unwrap()();
    clock.advance(Duration::from_millis(1));

    assert!(weak.unwrap().upgrade().is_none());
}

#[test]
fn something_else_can_take_the_request_instead() {
    let harness = Harness::new();
    let asked = Rc::new(Cell::new(0));
    {
        let asked = Rc::clone(&asked);
        harness.host.redirect_with(move || {
            asked.set(asked.get() + 1);
            true
        });
    }

    harness.host.open().unwrap();

    assert_eq!(asked.get(), 1);
    assert!(!harness.host.is_open());
}

#[test]
fn a_redirect_that_declines_lets_the_window_open() {
    let harness = Harness::new();
    harness.host.redirect_with(|| false);

    harness.host.open().unwrap();

    assert!(harness.host.is_open());
}

#[test]
fn a_window_that_cant_be_made_leaves_nothing_open() {
    let clock = Rc::new(ManualClock::default());
    let make = Rc::new(|| Err::<FakeWindow, _>("no display".to_owned()));
    let host = WindowHost::new(make, clock);

    assert_eq!(host.open(), Err("no display".to_owned()));
    assert!(!host.is_open());
}
