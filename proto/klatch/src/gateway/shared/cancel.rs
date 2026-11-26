//! Cancelling a look that is waiting on the owner thread (#1562).
//!
//! The owner serves one request at a time, so a look queued behind a slow request waits for it. `look_cancellable` queues
//! the look like `handle` does, registers it under its call id, and waits for the owner's answer in short ticks, looking
//! at a flag each time. A cancel raises the flag (once) through the registry. The waiter then stops waiting and answers
//! `Cancelled`. A job the owner has not started yet sees the flag and does nothing, so a request that never began has no
//! effect and writes nothing. A job the owner is already running finishes on its own, with every check and audit row it
//! passes, and its answer is dropped: nothing the cancel does reaches into the owner, and nothing is released that the
//! look does not own (a look holds no claim, no cursor, no lease).

use super::{Backend, Job, Principal, Work};
use crate::protocol::ServerFrame;
use crate::session::{CancelHook, Outcome};
use crate::sync::mpsc::RecvTimeoutError;
use crate::sync::{mpsc, recv_within};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// How often a waiting look looks at its flag. A cancel is seen within one tick.
const TICK: Duration = Duration::from_millis(20);

/// The looks of one connection that are waiting or running, by call id, and the flag of each.
#[derive(Default)]
pub(super) struct InFlight(Mutex<HashMap<String, Arc<AtomicBool>>>);

impl InFlight {
    fn map(&self) -> MutexGuard<'_, HashMap<String, Arc<AtomicBool>>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Register `flag` under `id`, unless a look already has that id (then this one is simply not cancellable, and a
    /// cancel of the id reaches only the look that holds it). The entry goes when the returned guard drops.
    fn enter(self: &Arc<Self>, id: &str, flag: &Arc<AtomicBool>) -> Entry {
        let mut map = self.map();
        let held = !map.contains_key(id);
        if held {
            map.insert(id.to_string(), flag.clone());
        }
        Entry {
            table: held.then(|| (self.clone(), id.to_string(), flag.clone())),
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.map().len()
    }
}

impl CancelHook for InFlight {
    fn cancel(&self, call_id: &str) -> bool {
        // `swap` makes the first cancel the only one that reports success.
        self.map()
            .get(call_id)
            .is_some_and(|flag| !flag.swap(true, Ordering::SeqCst))
    }
}

/// A registered look; removes exactly its own entry when it ends, however it ends.
struct Entry {
    table: Option<(Arc<InFlight>, String, Arc<AtomicBool>)>,
}

impl Drop for Entry {
    fn drop(&mut self) {
        if let Some((table, id, flag)) = self.table.take() {
            let mut map = table.map();
            if map.get(&id).is_some_and(|f| Arc::ptr_eq(f, &flag)) {
                map.remove(&id);
            }
        }
    }
}

impl super::SharedBackend {
    /// How many looks are in flight on this handle.
    #[cfg(test)]
    pub(crate) fn in_flight(&self) -> usize {
        self.inflight.len()
    }

    /// `handle` that a cancel under `call.id` ends early; see the module.
    pub(super) fn look_cancellable(&self, who: &Principal, call: crate::session::Call) -> Outcome {
        let id = call.id.clone();
        let flag = Arc::new(AtomicBool::new(false));
        let _entry = self.inflight.enter(&id, &flag);
        let (reply_tx, reply_rx) = mpsc::channel();
        let (run_who, run_flag) = (who.clone(), flag.clone());
        let job = Job {
            principal: who.clone(),
            keyed: true,
            harness: self.harness.clone(),
            run: Work::Backend(Box::new(move |b: &dyn Backend, h| {
                // Cancelled before the owner got to it: nothing is run, so nothing is checked, read or written.
                let outcome = if run_flag.load(Ordering::SeqCst) {
                    Outcome::Cancelled
                } else {
                    Outcome::Done(b.handle_as(&run_who, call, h))
                };
                let _ = reply_tx.send(outcome);
            })),
        };
        let sent = self.gateway.tx.send(job);
        // any request may have stored something: a waiting session's change token moves after it
        let finish = |outcome| {
            self.gateway.posts.fetch_add(1, Ordering::SeqCst);
            outcome
        };
        if sent.is_err() {
            return finish(Outcome::Done(failed_frames(&id)));
        }
        loop {
            match recv_within(&reply_rx, TICK) {
                Ok(outcome) => return finish(outcome),
                Err(RecvTimeoutError::Timeout) if flag.load(Ordering::SeqCst) => {
                    return finish(Outcome::Cancelled);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return finish(Outcome::Done(failed_frames(&id)));
                }
            }
        }
    }
}

fn failed_frames(id: &str) -> Vec<ServerFrame> {
    super::SharedBackend::failed(id.to_string(), &super::gone())
}
