//! Time, which only the front end's run loop can tell. Everything here runs on the UI thread.

use std::time::Duration;

/// A timer that does nothing once dropped.
pub trait Timer {
    /// Fires once after `delay`, in place of whatever was running.
    fn once(&self, delay: Duration, fire: Box<dyn FnOnce()>);
    /// Fires every `interval`, in place of whatever was running.
    fn every(&self, interval: Duration, fire: Box<dyn FnMut()>);
    /// Keeps firing what [`Timer::every`] started, at this rate from now on.
    fn set_interval(&self, interval: Duration);
    fn stop(&self);
    /// Whether something is still to fire.
    fn running(&self) -> bool {
        true
    }
}

pub trait Clock {
    fn timer(&self) -> Box<dyn Timer>;
    /// Runs `fire` after `delay`, with nothing left to cancel it.
    fn later(&self, delay: Duration, fire: Box<dyn FnOnce()>);
}
