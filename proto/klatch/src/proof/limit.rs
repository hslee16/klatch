//! The failure limit on the admin proof (#1233): a count of attempts and the time of the last one, kept in policy
//! (`admin-passphrase-fails`, `<count>,<unix secs>`), never the attempt itself.
//!
//! An attempt is CHARGED before it is checked, in one write transaction: a process that is killed after it was
//! typed, or ten processes guessing at once, each still cost one. Only a success clears the count. The first
//! `FREE` attempts are free; after that each must wait twice as long as the one before, up to `MAX_LOCK_SECS`.

use crate::domain::Result;
use crate::store::areas::PolicyStore;
use std::cell::Cell;

pub const KEY: &str = "admin-passphrase-fails";
/// Attempts that cost no waiting.
pub const FREE: u32 = 3;
const BASE_LOCK_SECS: i64 = 30;
pub const MAX_LOCK_SECS: i64 = 900;

/// How long the next attempt must wait after `count` charged attempts.
pub fn lock_secs(count: u32) -> i64 {
    if count < FREE {
        return 0;
    }
    let doublings = (count - FREE).min(10);
    (BASE_LOCK_SECS << doublings).min(MAX_LOCK_SECS)
}

/// The state as stored: `(count, time of the last charged attempt)`. A damaged value reads as the strictest wait
/// from `now`, so damage never grants an attempt.
fn read<S: PolicyStore + ?Sized>(store: &S, now: i64) -> Result<(u32, i64)> {
    Ok(match store.policy_get(KEY)? {
        None => (0, 0),
        Some(v) => v
            .split_once(',')
            .and_then(|(c, t)| Some((c.parse().ok()?, t.parse().ok()?)))
            .unwrap_or((u32::MAX, now)),
    })
}

pub(super) fn remaining(count: u32, last: i64, now: i64) -> i64 {
    // a clock that went back must not shorten the wait or lengthen it past the cap
    (last + lock_secs(count) - now).clamp(0, MAX_LOCK_SECS)
}

/// Seconds the next attempt must wait, 0 when it may be made now.
pub fn wait_secs<S: PolicyStore + ?Sized>(store: &S, now: i64) -> Result<i64> {
    let (count, last) = read(store, now)?;
    Ok(remaining(count, last, now))
}

/// Charge one attempt, atomically. `Ok(Ok(count))` is the count including this attempt; `Ok(Err(secs))` means the
/// wait is not over and nothing was charged.
pub fn charge<S: PolicyStore + ?Sized>(
    store: &S,
    now: i64,
) -> Result<std::result::Result<u32, i64>> {
    let outcome = Cell::new(Ok(0u32));
    store.atomically(&mut || {
        let (count, last) = read(store, now)?;
        let wait = remaining(count, last, now);
        if wait > 0 {
            outcome.set(Err(wait));
            return Ok(());
        }
        let count = count.saturating_add(1);
        store.policy_set(KEY, Some(&format!("{count},{now}")), now)?;
        outcome.set(Ok(count));
        Ok(())
    })?;
    Ok(outcome.get())
}

/// A success: forget the failures.
pub fn clear<S: PolicyStore + ?Sized>(store: &S, now: i64) -> Result<()> {
    store.policy_set(KEY, None, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_is_free_then_doubles_up_to_the_cap() {
        assert_eq!(lock_secs(0), 0);
        assert_eq!(lock_secs(FREE - 1), 0);
        assert_eq!(lock_secs(FREE), 30);
        assert_eq!(lock_secs(FREE + 1), 60);
        assert_eq!(lock_secs(FREE + 2), 120);
        assert_eq!(lock_secs(FREE + 20), MAX_LOCK_SECS);
        assert_eq!(lock_secs(u32::MAX), MAX_LOCK_SECS);
    }
}
