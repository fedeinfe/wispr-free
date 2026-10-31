//! Time that moves only when a test says so.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use wf_shell::clock::{Clock, Timer};

enum Fire {
    Idle,
    Once(Box<dyn FnOnce()>),
    Every(Box<dyn FnMut()>),
}

struct Slot {
    due: Option<Duration>,
    interval: Duration,
    fire: Fire,
}

#[derive(Default)]
struct Slots {
    now: Cell<Duration>,
    slots: RefCell<Vec<Rc<RefCell<Slot>>>>,
}

#[derive(Default)]
pub struct ManualClock {
    slots: Rc<Slots>,
}

impl ManualClock {
    fn slot(&self) -> Rc<RefCell<Slot>> {
        let slot = Rc::new(RefCell::new(Slot {
            due: None,
            interval: Duration::ZERO,
            fire: Fire::Idle,
        }));
        self.slots.slots.borrow_mut().push(Rc::clone(&slot));
        slot
    }

    /// Fires everything due by then, in order.
    pub fn advance(&self, by: Duration) {
        let until = self.slots.now.get() + by;
        loop {
            let next = self.slots.slots.borrow().iter().fold(None, |best, slot| {
                let due = slot.borrow().due.filter(|due| *due <= until);
                match (due, &best) {
                    (Some(due), Some((known, _))) if due >= *known => best,
                    (Some(due), _) => Some((due, Rc::clone(slot))),
                    _ => best,
                }
            });
            let Some((due, slot)) = next else {
                break;
            };
            self.slots.now.set(due);
            let fire = std::mem::replace(&mut slot.borrow_mut().fire, Fire::Idle);
            slot.borrow_mut().due = None;
            match fire {
                Fire::Idle => {}
                Fire::Once(fire) => fire(),
                Fire::Every(mut fire) => {
                    fire();
                    let mut slot = slot.borrow_mut();
                    // Whatever it fired may have started something else on this timer.
                    if matches!(slot.fire, Fire::Idle) && slot.due.is_none() {
                        slot.due = Some(due + slot.interval);
                        slot.fire = Fire::Every(fire);
                    }
                }
            }
        }
        self.slots.now.set(until);
    }
}

impl Clock for ManualClock {
    fn timer(&self) -> Box<dyn Timer> {
        Box::new(ManualTimer {
            slot: self.slot(),
            slots: Rc::clone(&self.slots),
        })
    }

    fn later(&self, delay: Duration, fire: Box<dyn FnOnce()>) {
        self.timer().once(delay, fire);
    }
}

struct ManualTimer {
    slot: Rc<RefCell<Slot>>,
    slots: Rc<Slots>,
}

impl Timer for ManualTimer {
    fn once(&self, delay: Duration, fire: Box<dyn FnOnce()>) {
        let mut slot = self.slot.borrow_mut();
        slot.due = Some(self.slots.now.get() + delay);
        slot.fire = Fire::Once(fire);
    }

    fn every(&self, interval: Duration, fire: Box<dyn FnMut()>) {
        let mut slot = self.slot.borrow_mut();
        slot.interval = interval;
        slot.due = Some(self.slots.now.get() + interval);
        slot.fire = Fire::Every(fire);
    }

    fn set_interval(&self, interval: Duration) {
        let mut slot = self.slot.borrow_mut();
        // It may be firing, and what it fired may be what asks: it goes on at this rate.
        slot.interval = interval;
        if matches!(slot.fire, Fire::Every(_)) {
            slot.due = Some(self.slots.now.get() + interval);
        }
    }

    fn stop(&self) {
        let mut slot = self.slot.borrow_mut();
        slot.due = None;
        slot.fire = Fire::Idle;
    }
}
