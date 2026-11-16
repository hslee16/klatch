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
