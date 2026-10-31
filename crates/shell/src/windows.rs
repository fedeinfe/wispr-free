//! Keeps the one open window of a kind, and nothing at all while it's closed.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Duration;

use crate::clock::Clock;
use crate::memory;

/// Long enough for the OS window to be gone once the window is dropped. The memory spike on macOS
/// collected this long after closing.
pub const WINDOW_TEARDOWN: Duration = Duration::from_millis(600);

/// What a host asks of an open window.
pub trait Window {
    fn show(&self) -> Result<(), String>;
    fn hide(&self);
    /// Gives it the foreground, which an app without a Dock icon doesn't get.
    fn bring_to_front(&self);
    /// Runs `close` when the user closes it.
    fn on_close(&self, close: Box<dyn Fn()>);
    /// Starts what the window fills in once it's up.
    fn start(&self) {}
}

/// Builds a window, each time one opens.
pub type Make<W> = Rc<dyn Fn() -> Result<W, String>>;

/// Asked before the window opens. True means something else took the request.
pub type Redirect = Rc<RefCell<Option<Box<dyn Fn() -> bool>>>>;

struct Shared<W> {
    make: Make<W>,
    open: RefCell<Option<W>>,
    redirect: Redirect,
    clock: Rc<dyn Clock>,
}

pub struct WindowHost<W> {
    shared: Rc<Shared<W>>,
}

impl<W> Clone for WindowHost<W> {
    fn clone(&self) -> Self {
        Self {
            shared: Rc::clone(&self.shared),
        }
    }
}

impl<W: Window + 'static> WindowHost<W> {
    pub fn new(make: Make<W>, clock: Rc<dyn Clock>) -> Self {
        Self {
            shared: Rc::new(Shared {
                make,
                open: RefCell::default(),
                redirect: Rc::default(),
                clock,
            }),
        }
    }

    fn weak(&self) -> Weak<Shared<W>> {
        Rc::downgrade(&self.shared)
    }

    pub fn redirect_with(&self, redirect: impl Fn() -> bool + 'static) {
        *self.shared.redirect.borrow_mut() = Some(Box::new(redirect));
    }

    pub fn is_open(&self) -> bool {
        self.shared.open.borrow().is_some()
    }

    pub fn with_window<T>(&self, use_it: impl FnOnce(&W) -> T) -> Option<T> {
        self.shared.open.borrow().as_ref().map(use_it)
    }

    /// Opens the window, or brings it forward if it's open already.
    pub fn open(&self) -> Result<(), String> {
        let redirect = self.shared.redirect.borrow();
        if redirect.as_ref().is_some_and(|taken| taken()) {
            return Ok(());
        }
        drop(redirect);
        if let Some(window) = &*self.shared.open.borrow() {
            window.show()?;
            window.bring_to_front();
            return Ok(());
        }
        let window = (self.shared.make)()?;
        let host = self.weak();
        let clock = Rc::clone(&self.shared.clock);
        window.on_close(Box::new(move || {
            // Not from inside the window's own callback: the close drops the window.
            let host = host.clone();
            clock.later(
                Duration::ZERO,
                Box::new(move || {
                    if let Some(shared) = host.upgrade() {
                        WindowHost { shared }.close();
                    }
                }),
            );
        }));
        window.start();
        window.show()?;
        window.bring_to_front();
        *self.shared.open.borrow_mut() = Some(window);
        Ok(())
    }

    /// Drops the window and everything it holds, then returns the freed memory to the OS.
    pub fn close(&self) {
        let window = self.shared.open.borrow_mut().take();
        if let Some(window) = window {
            window.hide();
        }
        self.shared
            .clock
            .later(WINDOW_TEARDOWN, Box::new(memory::release_freed));
    }
}
