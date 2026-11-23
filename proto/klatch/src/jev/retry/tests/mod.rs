use super::*;
use std::cell::{Cell, RefCell};
use std::time::Duration;

pub(super) fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

pub(super) fn headers(after: Option<&str>, after_ms: Option<&str>) -> RetryHeaders {
    RetryHeaders {
        retry_after: after.map(String::from),
        retry_after_ms: after_ms.map(String::from),
    }
}

/// A clock that moves only when the sleeper sleeps, and records every sleep. Nothing really waits.
pub(super) struct World {
    now: Cell<Duration>,
    slept: RefCell<Vec<Duration>>,
    unit: Cell<f64>,
}
impl World {
    fn new(unit: f64) -> World {
        World {
            now: Cell::new(Duration::ZERO),
            slept: RefCell::new(vec![]),
            unit: Cell::new(unit),
        }
    }
}
impl Clock for World {
    fn now(&self) -> Duration {
        self.now.get()
    }
}
impl Sleeper for World {
    fn sleep(&self, d: Duration) {
        self.slept.borrow_mut().push(d);
        self.now.set(self.now.get() + d);
    }
}
impl Jitter for World {
    fn unit(&self) -> f64 {
        self.unit.get()
    }
}
pub(super) fn timing(w: &World) -> Timing<'_> {
    Timing::new(w, w, w)
}

pub(super) fn st(status: u16) -> JevTransportError {
    JevTransportError::status(status, "BODY-SENTINEL")
}

mod backoff;
mod headers;
