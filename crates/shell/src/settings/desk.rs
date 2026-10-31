//! Keeps [`Pages`] for `Shell`. They're built on `Rc`, so they live on a thread of their own for as
//! long as the run does, which is what lets downloads and moves outlive their window. Everything
//! reaches them as an order on that thread, and what they show goes out whole, once a batch of
//! orders is done.

use std::cell::RefCell;
use std::panic::{self, AssertUnwindSafe};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

use wf_core::settings::SettingsStore;
use wf_core::shortcut::Platform;
use wf_models::Catalog;

use super::page::SettingsPage;
use super::state::{Folded, PagesState};
use super::{Desktop, Pages, Paths, SettingsParts, Shown};
use crate::clock::{Clock, Timer};
use crate::onboarding::SetupPage;
use crate::onboarding::desk::{SetupDesk, SetupRecipe};

/// How long quitting waits for the pages to let go. A job past that dies with the process, and its
/// `.part` file stays for next time.
const STOP_LIMIT: Duration = Duration::from_secs(2);
/// More than any order takes, short of a stuck disk.
const ASK_LIMIT: Duration = Duration::from_secs(10);

type Work = Box<dyn FnOnce(&Pages) + Send>;
type SetupWork = Box<dyn FnOnce(&SetupDesk) + Send>;

enum Order {
    Work(Work),
    Setup(SetupWork),
    /// Other threads have reported something.
    Drain,
    /// Something else changed the settings.
    Follow,
    Page(SettingsPage),
    /// Says what the pages show, after everything sent before it and once it's gone out.
    Ask(Sender<PagesState>),
    Stop(Option<Sender<()>>),
}

/// What the pages are built from, on their thread, where an `Rc` can be made.
pub struct DeskParts {
    pub store: Arc<SettingsStore>,
    pub catalog: Arc<Catalog>,
    pub paths: Paths,
    pub platform: Platform,
    pub page: SettingsPage,
    pub make_desktop: Box<dyn FnOnce() -> Rc<dyn Desktop> + Send>,
    /// Runs on the thread with the whole of what shows, each time it changes.
    pub publish: Box<dyn Fn(PagesState) + Send>,
    /// Runs on the thread with the microphone test's level.
    pub level: Box<dyn Fn(f32) + Send>,
    pub setup: SetupRecipe,
}

/// The thread with the pages. Dropping this stops it.
pub struct Desk {
    orders: Sender<Order>,
    id: ThreadId,
}

impl Desk {
    pub fn start(parts: DeskParts) -> std::io::Result<Self> {
        let (orders, inbox) = mpsc::channel();
        let wake = orders.clone();
        let thread = thread::Builder::new()
            .name("settings-pages".into())
            .spawn(move || run(parts, &inbox, wake))?;
        Ok(Self {
            orders,
            id: thread.thread().id(),
        })
    }

    /// Runs `work` on the pages, after everything sent before it.
    pub fn work(&self, work: impl FnOnce(&Pages) + Send + 'static) {
        let _ = self.orders.send(Order::Work(Box::new(work)));
    }

    /// Runs `work` on onboarding, after everything sent before it.
    pub fn setup(&self, work: impl FnOnce(&SetupDesk) + Send + 'static) {
        let _ = self.orders.send(Order::Setup(Box::new(work)));
    }

    /// What onboarding shows, once it has done all it was told. `None` with no window open.
    pub fn setup_page(&self) -> Option<SetupPage> {
        let (reply, answer) = mpsc::channel();
        self.setup(move |setup| {
            let _ = reply.send(setup.page());
        });
        answer.recv_timeout(ASK_LIMIT).ok().flatten()
    }

    /// Says the settings moved. The pages show their own changes, so they don't hear of those.
    pub fn follow(&self) {
        if thread::current().id() != self.id {
            let _ = self.orders.send(Order::Follow);
        }
    }

    pub fn set_page(&self, page: SettingsPage) {
        let _ = self.orders.send(Order::Page(page));
    }

    /// What the pages show, once they've done all they were told and it has gone out.
    pub fn ask(&self) -> Option<PagesState> {
        let (reply, answer) = mpsc::channel();
        self.orders.send(Order::Ask(reply)).ok()?;
        answer.recv_timeout(ASK_LIMIT).ok()
    }

    /// Lets the pages go, which cancels what they're running.
    pub fn stop(&self) {
        let (done, stopped) = mpsc::channel();
        if self.orders.send(Order::Stop(Some(done))).is_ok() {
            let _ = stopped.recv_timeout(STOP_LIMIT);
        }
    }
}

impl Drop for Desk {
    fn drop(&mut self) {
        // The pages' own mailbox holds a sender, so the thread can't see this one go.
        let _ = self.orders.send(Order::Stop(None));
    }
}

/// Runs `work`, and lets the thread live through a panic in it: the app's Settings would otherwise
/// stay dead until the next start, over an index gone stale.
pub(crate) fn guarded(work: impl FnOnce()) {
    if panic::catch_unwind(AssertUnwindSafe(work)).is_err() {
        eprintln!("the settings pages panicked");
    }
}

fn run(parts: DeskParts, inbox: &Receiver<Order>, wake: Sender<Order>) {
    let DeskParts {
        store,
        catalog,
        paths,
        platform,
        page,
        make_desktop,
        publish,
        level,
        setup,
    } = parts;
    let folded = Rc::new(RefCell::new(Folded::new(page)));
    let view = {
        let folded = Rc::clone(&folded);
        Rc::new(move |shown: Shown| {
            if let Shown::MicrophoneLevel(value) = shown {
                level(value);
            }
            folded.borrow_mut().take(shown);
        })
    };
    let clock = Rc::new(DeskClock::default());
    let settings = SettingsParts {
        store,
        catalog,
        desktop: make_desktop(),
        // The store's watcher tells the rest of the run.
        on_change: Rc::new(|_| {}),
        paths,
        platform,
    };
    let pages = Pages::new(
        settings.clone(),
        view,
        clock.as_ref(),
        Arc::new(move || {
            let _ = wake.send(Order::Drain);
        }),
    );
    let setup = SetupDesk::new(setup, settings, Rc::clone(&pages), clock.clone());

    let mut shown = None;
    let send_out = |shown: &mut Option<PagesState>| {
        let state = folded.borrow().state();
        if shown.as_ref() != Some(&state) {
            *shown = Some(state.clone());
            publish(state);
        }
    };
    send_out(&mut shown);
    loop {
        let first = match clock.next_due() {
            Some(due) => inbox.recv_timeout(due.saturating_duration_since(Instant::now())),
            None => inbox.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        let order = match first {
            Ok(order) => order,
            Err(RecvTimeoutError::Timeout) => {
                clock.fire_due();
                send_out(&mut shown);
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => return,
        };
        for order in std::iter::once(order).chain(inbox.try_iter()) {
            match order {
                Order::Work(work) => guarded(|| work(&pages)),
                Order::Setup(work) => guarded(|| work(&setup)),
                Order::Drain => guarded(|| pages.drain_events()),
                Order::Follow => guarded(|| pages.follow()),
                Order::Page(page) => folded.borrow_mut().set_page(page),
                Order::Ask(reply) => {
                    send_out(&mut shown);
                    let _ = reply.send(folded.borrow().state());
                }
                Order::Stop(done) => {
                    // An open onboarding holds the pages too.
                    drop(setup);
                    drop(pages);
                    if let Some(done) = done {
                        let _ = done.send(());
                    }
                    return;
                }
            }
        }
        // A steady stream of orders, as a dictation sends, never lets the wait above time out.
        clock.fire_due();
        send_out(&mut shown);
    }
}

enum Fire {
    Once(Box<dyn FnOnce()>),
    Every(Box<dyn FnMut()>),
}

#[derive(Default)]
struct Alarm {
    due: Option<Instant>,
    fire: Option<Fire>,
    /// Set while a repeating alarm goes on, which it still does while it's firing.
    interval: Option<Duration>,
}

type Slot = Rc<RefCell<Alarm>>;

/// Time on the pages' thread: the loop wakes for the next alarm.
#[derive(Default)]
pub(crate) struct DeskClock {
    slots: RefCell<Vec<Weak<RefCell<Alarm>>>>,
    /// Alarms nobody holds, from [`Clock::later`].
    kept: RefCell<Vec<Box<dyn Timer>>>,
}

impl DeskClock {
    pub(crate) fn next_due(&self) -> Option<Instant> {
        let slots = self.slots.borrow();
        let dues = slots.iter().filter_map(Weak::upgrade);
        dues.filter_map(|slot| slot.borrow().due).min()
    }

    pub(crate) fn fire_due(&self) {
        let now = Instant::now();
        self.slots
            .borrow_mut()
            .retain(|slot| slot.strong_count() > 0);
        let slots: Vec<_> = self
            .slots
            .borrow()
            .iter()
            .filter_map(Weak::upgrade)
            .collect();
        for slot in slots {
            let fire = {
                let mut alarm = slot.borrow_mut();
                if alarm.due.is_some_and(|due| due <= now) {
                    alarm.due = None;
                    alarm.fire.take()
                } else {
                    None
                }
            };
            // Out of the borrow: what fires may set the same timer again.
            match fire {
                Some(Fire::Once(fire)) => guarded(fire),
                Some(Fire::Every(mut fire)) => {
                    guarded(&mut fire);
                    let mut alarm = slot.borrow_mut();
                    // What it fired may have stopped it, or started something else on this timer.
                    if let (None, Some(interval)) = (&alarm.fire, alarm.interval) {
                        alarm.due = Some(Instant::now() + interval);
                        alarm.fire = Some(Fire::Every(fire));
                    }
                }
                None => {}
            }
        }
        self.kept.borrow_mut().retain(|timer| timer.running());
    }
}

struct DeskTimer(Slot);

impl Timer for DeskTimer {
    fn once(&self, delay: Duration, fire: Box<dyn FnOnce()>) {
        *self.0.borrow_mut() = Alarm {
            due: Some(Instant::now() + delay),
            fire: Some(Fire::Once(fire)),
            interval: None,
        };
    }

    fn every(&self, interval: Duration, fire: Box<dyn FnMut()>) {
        *self.0.borrow_mut() = Alarm {
            due: Some(Instant::now() + interval),
            fire: Some(Fire::Every(fire)),
            interval: Some(interval),
        };
    }

    fn set_interval(&self, interval: Duration) {
        let mut alarm = self.0.borrow_mut();
        if alarm.interval.is_some() {
            alarm.interval = Some(interval);
            if alarm.due.is_some() {
                alarm.due = Some(Instant::now() + interval);
            }
        }
    }

    fn stop(&self) {
        *self.0.borrow_mut() = Alarm::default();
    }

    fn running(&self) -> bool {
        self.0.borrow().due.is_some()
    }
}

impl Clock for DeskClock {
    fn timer(&self) -> Box<dyn Timer> {
        let slot = Slot::default();
        self.slots.borrow_mut().push(Rc::downgrade(&slot));
        Box::new(DeskTimer(slot))
    }

    fn later(&self, delay: Duration, fire: Box<dyn FnOnce()>) {
        let timer = self.timer();
        timer.once(delay, fire);
        self.kept.borrow_mut().push(timer);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    const SOON: Duration = Duration::from_millis(1);
    const LATER: Duration = Duration::from_secs(3600);

    fn counter() -> (Rc<Cell<u32>>, Rc<Cell<u32>>) {
        let count = Rc::new(Cell::new(0));
        (Rc::clone(&count), count)
    }

    fn fire_after_due(clock: &DeskClock) {
        thread::sleep(SOON * 2);
        clock.fire_due();
    }

    #[test]
    fn an_alarm_fires_once_at_its_time() {
        let clock = DeskClock::default();
        let timer = clock.timer();
        let (count, seen) = counter();

        timer.once(SOON, Box::new(move || count.set(count.get() + 1)));
        assert!(clock.next_due().is_some());
        fire_after_due(&clock);
        fire_after_due(&clock);

        assert_eq!(seen.get(), 1);
        assert_eq!(clock.next_due(), None);
    }

    #[test]
    fn an_alarm_not_due_stays() {
        let clock = DeskClock::default();
        let timer = clock.timer();
        let (count, seen) = counter();

        timer.once(LATER, Box::new(move || count.set(count.get() + 1)));
        clock.fire_due();

        assert_eq!(seen.get(), 0);
        assert!(clock.next_due().is_some());
    }

    #[test]
    fn a_stopped_alarm_and_a_dropped_timer_do_nothing() {
        let clock = DeskClock::default();
        let (stopped, dropped) = (clock.timer(), clock.timer());
        let (count, seen) = counter();
        let other = Rc::clone(&count);

        stopped.once(SOON, Box::new(move || count.set(1)));
        dropped.once(SOON, Box::new(move || other.set(1)));
        stopped.stop();
        drop(dropped);
        fire_after_due(&clock);

        assert_eq!(seen.get(), 0);
        assert_eq!(clock.next_due(), None);
    }

    #[test]
    fn a_repeating_alarm_goes_on_until_stopped() {
        let clock = DeskClock::default();
        let timer = clock.timer();
        let (count, seen) = counter();

        timer.every(SOON, Box::new(move || count.set(count.get() + 1)));
        fire_after_due(&clock);
        fire_after_due(&clock);
        assert_eq!(seen.get(), 2);

        timer.stop();
        fire_after_due(&clock);
        assert_eq!(seen.get(), 2);
    }

    #[test]
    fn a_new_rate_counts_from_now_and_not_from_the_old_alarm() {
        let clock = DeskClock::default();
        let timer = clock.timer();
        let (count, seen) = counter();

        timer.every(LATER, Box::new(move || count.set(count.get() + 1)));
        timer.set_interval(SOON);
        fire_after_due(&clock);

        assert_eq!(seen.get(), 1);
    }

    /// Onboarding opens from inside its own look, and that's what asks for the faster rate.
    #[test]
    fn a_repeating_alarm_can_change_its_rate_and_stop_itself_while_it_fires() {
        let clock = DeskClock::default();
        let timer: Rc<dyn Timer> = Rc::from(clock.timer());
        let (count, seen) = counter();

        let weak = Rc::downgrade(&timer);
        timer.every(
            SOON,
            Box::new(move || {
                count.set(count.get() + 1);
                if let Some(timer) = weak.upgrade() {
                    match count.get() {
                        1 => timer.set_interval(LATER),
                        _ => timer.stop(),
                    }
                }
            }),
        );
        fire_after_due(&clock);
        assert!(clock.next_due().unwrap() > Instant::now() + LATER / 2);
        fire_after_due(&clock);
        assert_eq!(seen.get(), 1, "the new rate holds");

        timer.set_interval(SOON);
        fire_after_due(&clock);
        assert_eq!(seen.get(), 2);
        assert_eq!(clock.next_due(), None, "it stopped itself");
    }

    #[test]
    fn an_alarm_can_set_its_own_timer_again() {
        let clock = Rc::new(DeskClock::default());
        let timer: Rc<dyn Timer> = Rc::from(clock.timer());
        let (count, seen) = counter();

        let again = Rc::clone(&timer);
        timer.once(
            SOON,
            Box::new(move || {
                count.set(1);
                again.once(LATER, Box::new(|| {}));
            }),
        );
        fire_after_due(&clock);

        assert_eq!(seen.get(), 1);
        assert!(clock.next_due().is_some());
    }

    #[test]
    fn an_alarm_nobody_holds_still_fires_and_is_let_go() {
        let clock = DeskClock::default();
        let (count, seen) = counter();

        clock.later(SOON, Box::new(move || count.set(1)));
        fire_after_due(&clock);

        assert_eq!(seen.get(), 1);
        assert!(clock.kept.borrow().is_empty());
    }

    #[test]
    fn an_alarm_that_panics_leaves_the_clock_working() {
        let clock = DeskClock::default();
        let timer = clock.timer();
        let (count, seen) = counter();

        timer.once(SOON, Box::new(|| panic!("on purpose")));
        fire_after_due(&clock);
        timer.once(SOON, Box::new(move || count.set(1)));
        fire_after_due(&clock);

        assert_eq!(seen.get(), 1);
    }
}
