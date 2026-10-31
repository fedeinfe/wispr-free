//! One open Meetings window behind `Shell`. [`Meetings`] is built on `Rc`, so it lives on a thread
//! of its own, made when the window opens and gone when it closes, with everything it read. All
//! that reaches it is an order on that thread, so the window's calls never wait and stay in order,
//! and the disk work behind them stays on [`Meetings`]' own disk thread. What it shows goes out
//! once a batch of orders is done, so a meeting opening lands in Swift as one change.

use std::cell::RefCell;
use std::io;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use wf_dictation::MeetingNews;

use super::changes::MeetingsChange;
use super::{Meetings, MeetingsParts};
use crate::settings::desk::{DeskClock, guarded};

/// How long the window's thread waits for its disk work. A disk that takes longer than this keeps
/// going by itself, unless the app is on its way out.
const DISK_LIMIT: Duration = Duration::from_secs(2);
/// How long anything waits for the thread to go, past its wait for the disk.
const STOP_LIMIT: Duration = Duration::from_secs(3);

type Work = Box<dyn FnOnce(&Meetings) + Send>;

enum Order {
    Work(Work),
    /// The disk thread has reported something.
    Drain,
    Stop,
}

/// What the window is built from, on its thread, where an `Rc` can be made.
pub struct DeskParts {
    pub make_parts: Box<dyn FnOnce() -> MeetingsParts + Send>,
    /// Runs on the thread with each batch of changes.
    pub publish: Box<dyn Fn(Vec<MeetingsChange>) + Send>,
}

/// The thread with the open window.
pub struct MeetingsDesk {
    orders: Sender<Order>,
    gone: Receiver<()>,
}

impl MeetingsDesk {
    pub fn start(parts: DeskParts) -> io::Result<Self> {
        let (orders, inbox) = mpsc::channel();
        let (going, gone) = mpsc::channel::<()>();
        let wake = orders.clone();
        thread::Builder::new()
            .name("meetings-window".into())
            .spawn(move || {
                let _going = going;
                run(parts, &inbox, wake);
            })?;
        Ok(Self { orders, gone })
    }

    /// Runs `work` on the window, after everything sent before it.
    pub fn work(&self, work: impl FnOnce(&Meetings) + Send + 'static) {
        let _ = self.orders.send(Order::Work(Box::new(work)));
    }

    /// Takes in what the flow reported.
    pub fn news(&self, news: MeetingNews) {
        self.work(move |meetings| meetings.news(news));
    }

    /// Saves what's unsaved and lets the window go. It doesn't wait: see [`MeetingsDesk::wait`].
    pub fn stop(&self) {
        let _ = self.orders.send(Order::Stop);
    }

    /// Returns once a stopped window is gone, with its saves on disk.
    pub fn wait(&self) {
        let _ = self.gone.recv_timeout(STOP_LIMIT);
    }
}

impl Drop for MeetingsDesk {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(parts: DeskParts, inbox: &Receiver<Order>, wake: Sender<Order>) {
    let DeskParts {
        make_parts,
        publish,
    } = parts;
    let waiting: Rc<RefCell<Vec<MeetingsChange>>> = Rc::default();
    let view = {
        let waiting = Rc::clone(&waiting);
        Rc::new(move |shown: super::Shown| waiting.borrow_mut().push(shown.into()))
    };
    let clock = Rc::new(DeskClock::default());
    let wake = Arc::new(move || {
        let _ = wake.send(Order::Drain);
    });
    let meetings = match Meetings::new(make_parts(), view, clock.clone(), wake) {
        Ok(meetings) => meetings,
        Err(error) => {
            eprintln!("could not open the meetings: {error}");
            return;
        }
    };
    let send_out = || {
        let changes = waiting.take();
        if !changes.is_empty() {
            publish(changes);
        }
    };
    send_out();
    loop {
        let first = match clock.next_due() {
            Some(due) => inbox.recv_timeout(due.saturating_duration_since(Instant::now())),
            None => inbox.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        let order = match first {
            Ok(order) => order,
            Err(RecvTimeoutError::Timeout) => {
                clock.fire_due();
                send_out();
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => return,
        };
        for order in std::iter::once(order).chain(inbox.try_iter()) {
            match order {
                Order::Work(work) => guarded(|| work(&meetings)),
                Order::Drain => guarded(|| meetings.drain_events()),
                Order::Stop => {
                    let saved = meetings.finish_up();
                    drop(meetings);
                    let _ = saved.recv_timeout(DISK_LIMIT);
                    return;
                }
            }
        }
        // A steady stream of orders, as an answer's tokens are, never lets the wait above time out.
        clock.fire_due();
        send_out();
    }
}
