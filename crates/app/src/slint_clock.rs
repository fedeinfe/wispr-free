//! Time for `wf-shell`, from Slint's run loop.

use std::cell::{Cell, RefCell};
use std::time::Duration;

use slint::{Timer, TimerMode};
use wf_shell::clock;

pub struct SlintClock;

impl clock::Clock for SlintClock {
    fn timer(&self) -> Box<dyn clock::Timer> {
        Box::new(SlintTimer::default())
    }

    fn later(&self, delay: Duration, fire: Box<dyn FnOnce()>) {
        Timer::single_shot(delay, fire);
    }
}

#[derive(Default)]
struct SlintTimer {
    timer: Timer,
}

impl clock::Timer for SlintTimer {
    fn once(&self, delay: Duration, fire: Box<dyn FnOnce()>) {
        let fire = Cell::new(Some(fire));
        self.timer.start(TimerMode::SingleShot, delay, move || {
            if let Some(fire) = fire.take() {
                fire();
            }
        });
    }

    fn every(&self, interval: Duration, fire: Box<dyn FnMut()>) {
        let fire = RefCell::new(fire);
        self.timer.start(TimerMode::Repeated, interval, move || {
            (fire.borrow_mut())();
        });
    }

    fn set_interval(&self, interval: Duration) {
        self.timer.set_interval(interval);
    }

    fn stop(&self) {
        self.timer.stop();
    }
}
