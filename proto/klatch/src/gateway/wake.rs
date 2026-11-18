//! Waiting for the store to change (#616, #148): a waiter sleeps in short ticks and looks at the store's change token,
//! so it wakes within a tick of a commit instead of after a whole poll period. The token only says "something was
//! committed"; what a wake shows and every check on it stay with the poll that follows. A store with no token, or a
//! change that never comes, ends the pause at the fallback limit, which is the old fixed poll.

use std::time::{Duration, Instant};

/// How often the token is looked at. A look is one cheap pragma, not a query on the tables.
pub const WAKE_TICK: Duration = Duration::from_millis(20);

/// The longest a waiter sleeps without polling the tables: the old fixed poll period, kept as the fallback.
pub const WAKE_FALLBACK: Duration = Duration::from_millis(200);

/// Why `pause_for_change` returned.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Pause {
    /// The token differs from the one the caller took before it polled.
    Changed,
    /// The limit passed with no change seen (or the store has no token).
    Elapsed,
}

/// Sleep until `token()` differs from `seen` or `limit` has passed, looking every `tick`. Take `seen` before the poll
/// it follows, so a commit that lands after the token and before the poll shows up as a change here and is polled
/// again at once.
pub fn pause_for_change(
    token: &dyn Fn() -> Option<u64>,
    seen: Option<u64>,
    limit: Duration,
    tick: Duration,
) -> Pause {
    let end = Instant::now() + limit;
    loop {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Pause::Elapsed;
        }
        std::thread::sleep(tick.min(left));
        if seen.is_some() && token() != seen {
            return Pause::Changed;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    const TICK: Duration = Duration::from_millis(5);

    #[test]
    fn a_change_ends_the_pause_long_before_the_limit() {
        let counter = Arc::new(AtomicU64::new(1));
        let bump = counter.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            bump.store(2, Ordering::SeqCst);
        });
        let started = Instant::now();
        let read = counter.clone();
        let why = pause_for_change(
            &move || Some(read.load(Ordering::SeqCst)),
            Some(1),
            Duration::from_secs(5),
            TICK,
        );
        assert_eq!(why, Pause::Changed);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn no_change_waits_the_whole_limit() {
        let started = Instant::now();
        let why = pause_for_change(&|| Some(7), Some(7), Duration::from_millis(60), TICK);
        assert_eq!(why, Pause::Elapsed);
        assert!(started.elapsed() >= Duration::from_millis(60));
    }

    #[test]
    fn a_store_with_no_token_waits_the_whole_limit() {
        let started = Instant::now();
        let why = pause_for_change(&|| None, None, Duration::from_millis(60), TICK);
        assert_eq!(why, Pause::Elapsed);
        assert!(started.elapsed() >= Duration::from_millis(60));
    }

    #[test]
    fn a_token_that_was_already_stale_returns_at_the_first_tick() {
        let started = Instant::now();
        let why = pause_for_change(&|| Some(2), Some(1), Duration::from_secs(5), TICK);
        assert_eq!(why, Pause::Changed);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn the_pause_never_overshoots_a_short_limit_by_a_tick() {
        let started = Instant::now();
        pause_for_change(
            &|| Some(1),
            Some(1),
            Duration::from_millis(10),
            Duration::from_secs(1),
        );
        assert!(started.elapsed() < Duration::from_millis(500));
    }
}
