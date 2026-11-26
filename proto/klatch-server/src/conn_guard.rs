//! The global connection slot of the accept loops (#1640): `serve_concurrent` (the http and https listeners) and the
//! daemon's Unix socket loop take one slot per open connection, and a slot is a guard. [`Slot`] gives its place back in
//! `Drop`, so it is freed on every way out of the connection: a normal return, an early return, and an unwind. Nothing
//! decrements by hand after a handler returns. [`run`] also catches a handler that panics, so one bad connection costs
//! one connection: the thread ends, the slot comes back, the accept loop and the process go on, and the log gets one
//! fixed line that carries no data.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;

/// The open connections by number: the handle of each (so that stopping can shut them all down) and the count (the cap).
pub struct Registry<S>(Mutex<HashMap<u64, S>>);

impl<S> Default for Registry<S> {
    fn default() -> Self {
        Registry(Mutex::default())
    }
}

/// One connection's place in a [`Registry`]; dropping it removes the handle and so frees the place.
pub struct Slot<'a, S> {
    registry: &'a Registry<S>,
    id: u64,
}

impl<S> Registry<S> {
    /// Take a place for `handle` unless `max` are open. `next` is the connection counter; the handle comes back on a
    /// refusal so the caller can still answer the connection.
    pub fn claim<'a>(&'a self, max: usize, next: &mut u64, handle: S) -> Result<Slot<'a, S>, S> {
        let mut open = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if open.len() >= max {
            return Err(handle);
        }
        *next += 1;
        open.insert(*next, handle);
        Ok(Slot {
            registry: self,
            id: *next,
        })
    }

    /// Call `f` on every open connection's handle (stopping shuts each down).
    pub fn each(&self, mut f: impl FnMut(&S)) {
        for s in self.0.lock().unwrap_or_else(|e| e.into_inner()).values() {
            f(s);
        }
    }

    /// How many connections are open.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

impl<S> Drop for Slot<'_, S> {
    fn drop(&mut self) {
        self.registry
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
    }
}

/// The one line a panicking handler leaves in the log. Fixed: no message, no address, no data.
pub const PANIC_LINE: &str = "klatch-server: handler panicked";

/// Serve a connection holding `slot`. The slot is released when this returns, by return or by unwind alike. A panic in
/// `serve` is caught here (a panicking thread of a `thread::scope` would otherwise re-panic out of the scope and end the
/// accept loop), logged as [`PANIC_LINE`], and the caller's thread ends normally.
pub fn run<S>(slot: Slot<'_, S>, serve: impl FnOnce()) {
    let _slot = slot;
    if catch_unwind(AssertUnwindSafe(serve)).is_err() {
        eprintln!("{PANIC_LINE}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slot_is_given_back_when_the_work_returns_or_panics() {
        let reg: Registry<()> = Registry::default();
        let mut next = 0;
        let a = reg.claim(1, &mut next, ()).ok().unwrap();
        assert!(reg.claim(1, &mut next, ()).is_err());
        run(a, || {});
        assert_eq!(reg.len(), 0);
        let b = reg.claim(1, &mut next, ()).ok().unwrap();
        run(b, || panic!("boom"));
        assert_eq!(reg.len(), 0, "a panic keeps the slot");
        assert!(reg.claim(1, &mut next, ()).is_ok());
    }

    #[test]
    fn a_panic_in_a_scoped_helper_thread_is_caught_too() {
        // the https handler runs its application on a scoped thread (`tls.rs`); that panic re-panics out of the scope
        // into the handler, so catching at the handler's top is enough
        let reg: Registry<()> = Registry::default();
        let mut next = 0;
        let slot = reg.claim(1, &mut next, ()).ok().unwrap();
        run(slot, || {
            std::thread::scope(|s| {
                s.spawn(|| panic!("inner"));
            });
        });
        assert_eq!(reg.len(), 0);
    }
}
