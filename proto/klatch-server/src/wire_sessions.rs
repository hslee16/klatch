//! The registry of open `GET /v1/stream` sessions (#1643): the table `POST /v1/frames` looks a session up in. An entry is
//! created in exactly one place, [`register`], and removed in exactly one place, the `Drop` of the [`Registration`] that
//! `register` returns. The registration is held by the session's own code and by its closer, so the entry goes on every way
//! out: a normal close, an early return, and an unwind from a panic. Nothing removes an entry by hand,
//! and a drop removes only the entry its own registration made (#1667).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};

/// Hands out a number to each registration, so that a drop removes its own entry and never another that took its id.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// How many posted bodies may wait in a session's input.
pub(crate) const INBOX: usize = 16;

/// A session that is open: whose it is, and where its input goes.
pub(crate) struct Entry {
    pub(crate) member: String,
    pub(crate) tx: SyncSender<Vec<u8>>,
    /// Which registration made this entry.
    generation: u64,
}

pub(crate) type Sessions = Arc<Mutex<HashMap<String, Entry>>>;

/// A session's place in the registry: dropping it removes the entry, which ends what `POST /v1/frames` can reach.
pub(crate) struct Registration {
    sessions: Sessions,
    id: String,
    generation: u64,
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut table = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        if table
            .get(&self.id)
            .is_some_and(|e| e.generation == self.generation)
        {
            table.remove(&self.id);
        }
    }
}

/// Register a new session `id` of `member`: its registration, and the receiving end of its input.
pub(crate) fn register(
    sessions: &Sessions,
    id: &str,
    member: &str,
) -> (Registration, Receiver<Vec<u8>>) {
    let (tx, rx) = sync_channel::<Vec<u8>>(INBOX);
    let generation = GENERATION.fetch_add(1, Ordering::Relaxed);
    let entry = Entry {
        member: member.to_string(),
        tx,
        generation,
    };
    sessions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.to_string(), entry);
    let registration = Registration {
        sessions: sessions.clone(),
        id: id.to_string(),
        generation,
    };
    (registration, rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    fn has(sessions: &Sessions, id: &str) -> bool {
        sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(id)
    }

    #[test]
    fn an_entry_goes_when_its_registration_is_dropped() {
        let sessions = Sessions::default();
        let (a, _rx_a) = register(&sessions, "a", "alice");
        let (b, _rx_b) = register(&sessions, "b", "bob");
        assert!(has(&sessions, "a") && has(&sessions, "b"));
        drop(a);
        assert!(
            !has(&sessions, "a"),
            "dropping the registration kept the entry"
        );
        assert!(has(&sessions, "b"), "another session's entry went too");
        drop(b);
        assert!(sessions.lock().unwrap().is_empty());
    }

    #[test]
    fn a_dropped_registration_never_removes_a_later_session_that_took_its_id() {
        let sessions = Sessions::default();
        let (old, _rx_old) = register(&sessions, "s", "alice");
        let (new, _rx_new) = register(&sessions, "s", "bob");
        drop(old);
        assert!(
            has(&sessions, "s"),
            "an old registration removed a newer entry"
        );
        assert_eq!(sessions.lock().unwrap()["s"].member, "bob");
        drop(new);
        assert!(!has(&sessions, "s"));
    }

    #[test]
    fn an_entry_goes_when_the_session_panics_before_it_closes() {
        let sessions = Sessions::default();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let (registration, _rx) = register(&sessions, "s", "alice");
            assert!(has(&sessions, "s"));
            // the closer of the real session owns the registration, and a panic drops it unused
            let closer: Box<dyn FnOnce()> = Box::new(move || drop(registration));
            let _held = closer;
            panic!("a session panics before it closes");
        }));
        assert!(outcome.is_err());
        assert!(!has(&sessions, "s"), "a panicking session left its entry");
    }

    #[test]
    fn a_registration_poisoned_registry_is_still_cleaned() {
        let sessions = Sessions::default();
        let (registration, _rx) = register(&sessions, "s", "alice");
        let poisoner = sessions.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.lock().unwrap();
            panic!("poison the registry");
        })
        .join();
        drop(registration);
        assert!(!has(&sessions, "s"));
    }
}
