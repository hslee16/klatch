//! The thread primitives the shared-state code uses, in one place so a test build can swap them (#406).
//!
//! A normal build re-exports `std`. A build with `RUSTFLAGS="--cfg shuttle"` (only `proto/scripts/shuttle` sets it)
//! re-exports Shuttle's versions of the same names, so Shuttle controls the order of the threads' steps. Code that
//! shares state between threads imports from here instead of from `std::sync` and `std::thread`.

#[cfg(not(shuttle))]
pub use std::sync::{atomic, mpsc};
#[cfg(not(shuttle))]
pub use std::thread;

#[cfg(shuttle)]
pub use shuttle::sync::{atomic, mpsc};
#[cfg(shuttle)]
pub use shuttle::thread;

/// Run `probe` on its own thread and wait up to `limit` for its answer; `None` when it does not come. A keystore can
/// block for ever on a dialog nobody is there to answer, and a health check must not hang with it. The abandoned thread
/// ends with the process.
#[cfg(not(shuttle))]
pub fn within<T: Send + 'static>(
    limit: std::time::Duration,
    probe: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(probe());
    });
    rx.recv_timeout(limit).ok()
}

/// `within` in a Shuttle build. Shuttle has no clock: its `recv_timeout` never times out (#666), so a stuck probe would
/// leave the waiter blocked for ever, a deadlock Shuttle would report for code that is fine in real time. A second
/// thread stands in for the clock: it yields (Shuttle's `sleep`) and then sends the expiry on the same channel. The probe's
/// answer and the expiry reach the waiter in whichever order the schedule picks, so a test sees both outcomes (the
/// answer first, and the limit passing first with the answer arriving at a waiter that has gone). It does not model how
/// long `limit` is, only that the limit can pass at any step.
#[cfg(shuttle)]
pub fn within<T: Send + 'static>(
    limit: std::time::Duration,
    probe: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = mpsc::channel::<Option<T>>();
    let clock = tx.clone();
    thread::spawn(move || {
        let _ = tx.send(Some(probe()));
    });
    thread::spawn(move || {
        thread::sleep(limit);
        let _ = clock.send(None);
    });
    rx.recv().ok().flatten()
}

/// Wait up to `wait` for the next item on `rx`: `rx.recv_timeout(wait)`.
#[cfg(not(shuttle))]
pub fn recv_within<T>(
    rx: &mpsc::Receiver<T>,
    wait: std::time::Duration,
) -> Result<T, mpsc::RecvTimeoutError> {
    rx.recv_timeout(wait)
}

/// `recv_within` in a Shuttle build. Shuttle's `recv_timeout` never times out (#666), so a loop that wakes on a period
/// (the session's poll and heartbeat) would block on an idle input for ever. Here time may pass whenever nothing is
/// waiting: the waiter yields once, so the other threads can run first, then takes what has arrived or reports a
/// timeout. A schedule therefore sees both an answer that beats the period and a period that beats the answer. It
/// does not model how long `wait` is.
#[cfg(shuttle)]
pub fn recv_within<T>(
    rx: &mpsc::Receiver<T>,
    _wait: std::time::Duration,
) -> Result<T, mpsc::RecvTimeoutError> {
    thread::yield_now();
    match rx.try_recv() {
        Ok(item) => Ok(item),
        Err(mpsc::TryRecvError::Empty) => Err(mpsc::RecvTimeoutError::Timeout),
        Err(mpsc::TryRecvError::Disconnected) => Err(mpsc::RecvTimeoutError::Disconnected),
    }
}
