use std::time::{SystemTime, UNIX_EPOCH};

/// Where the gateway gets the time. Injected so behaviour is testable.
pub trait Clock {
    fn now(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64)
    }
}

/// A clock that always reports the same time; for tests and for callers that need determinism.
pub struct FixedClock(pub i64);

impl Clock for FixedClock {
    fn now(&self) -> i64 {
        self.0
    }
}
