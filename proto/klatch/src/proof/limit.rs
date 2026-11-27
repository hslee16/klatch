//! The failure limit on the admin proof (#1233): a count of attempts and the time of the last one, kept in policy
//! (`admin-passphrase-fails`, one `<terminal>=<count>,<unix secs>` entry per controlling terminal, #1274), never the
//! attempt itself. The count is per terminal, so a program with a pseudo-terminal of its own locks only that one.
//!
//! An attempt is CHARGED before it is checked, in one write transaction: a process that is killed after it was
//! typed, or ten processes guessing at once, each still cost one. Only a success clears the count. The first
//! `FREE` attempts are free; after that each must wait twice as long as the one before, up to `MAX_LOCK_SECS`.

use super::terminal;
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

/// Most terminals kept; the oldest go first.
const MAX_TERMINALS: usize = 64;
/// A terminal untouched for this long is forgotten (a person's terminal that has not failed for a week).
const IDLE_FORGET_SECS: i64 = 7 * 24 * 3600;

/// The state as stored: one `label=count,time of the last charged attempt` entry per terminal, separated by `;`.
/// A row from before #1274 (`count,time`) is read as the asking terminal's own entry, once. A damaged value reads
/// as the strictest wait from `now` for the asking terminal, so damage never grants an attempt.
fn read<S: PolicyStore + ?Sized>(
    store: &S,
    term: &str,
    now: i64,
) -> Result<Vec<(String, u32, i64)>> {
    let Some(v) = store.policy_get(KEY)? else {
        return Ok(Vec::new());
    };
    let damaged = || vec![(term.to_string(), u32::MAX, now)];
    if v.is_empty() {
        return Ok(Vec::new());
    }
    if !v.contains('=') {
        return Ok(match v.split_once(',') {
            Some((c, t)) => match (c.parse(), t.parse()) {
                (Ok(c), Ok(t)) => vec![(term.to_string(), c, t)],
                _ => damaged(),
            },
            None => damaged(),
        });
    }
    let mut out = Vec::new();
    for entry in v.split(';') {
        let parsed = entry.split_once('=').and_then(|(l, rest)| {
            let (c, t) = rest.split_once(',')?;
            Some((l.to_string(), c.parse().ok()?, t.parse().ok()?))
        });
        match parsed {
            Some(p) => out.push(p),
            None => return Ok(damaged()),
        }
    }
    Ok(out)
}

fn write<S: PolicyStore + ?Sized>(
    store: &S,
    mut entries: Vec<(String, u32, i64)>,
    now: i64,
) -> Result<()> {
    entries.retain(|(_, _, t)| now - *t <= IDLE_FORGET_SECS);
    if entries.len() > MAX_TERMINALS {
        entries.sort_by_key(|(_, _, t)| std::cmp::Reverse(*t));
        entries.truncate(MAX_TERMINALS);
    }
    if entries.is_empty() {
        return store.reserved_policy_set(KEY, None, now);
    }
    let text: Vec<String> = entries
        .iter()
        .map(|(l, c, t)| format!("{l}={c},{t}"))
        .collect();
    store.reserved_policy_set(KEY, Some(&text.join(";")), now)
}

fn of(entries: &[(String, u32, i64)], term: &str) -> (u32, i64) {
    entries
        .iter()
        .find(|(l, _, _)| l == term)
        .map(|(_, c, t)| (*c, *t))
        .unwrap_or((0, 0))
}

pub(super) fn remaining(count: u32, last: i64, now: i64) -> i64 {
    // a clock that went back must not shorten the wait or lengthen it past the cap
    last.saturating_add(lock_secs(count))
        .saturating_sub(now)
        .clamp(0, MAX_LOCK_SECS)
}

/// Seconds the next attempt from terminal `term` must wait, 0 when it may be made now.
pub fn wait_secs<S: PolicyStore + ?Sized>(store: &S, term: &str, now: i64) -> Result<i64> {
    let term = terminal::clean(term);
    let (count, last) = of(&read(store, &term, now)?, &term);
    Ok(remaining(count, last, now))
}

/// Charge one attempt from `term`, atomically. `Ok(Ok(count))` is that terminal's count including this attempt;
/// `Ok(Err(secs))` means its wait is not over and nothing was charged.
pub fn charge<S: PolicyStore + ?Sized>(
    store: &S,
    term: &str,
    now: i64,
) -> Result<std::result::Result<u32, i64>> {
    let term = terminal::clean(term);
    let outcome = Cell::new(Ok(0u32));
    store.atomically(&mut || {
        let mut entries = read(store, &term, now)?;
        let (count, last) = of(&entries, &term);
        let wait = remaining(count, last, now);
        if wait > 0 {
            outcome.set(Err(wait));
            return Ok(());
        }
        let count = count.saturating_add(1);
        entries.retain(|(l, _, _)| *l != term);
        entries.push((term.clone(), count, now));
        write(store, entries, now)?;
        outcome.set(Ok(count));
        Ok(())
    })?;
    Ok(outcome.get())
}

/// A success on `term`: forget that terminal's failures, and only those.
pub fn clear<S: PolicyStore + ?Sized>(store: &S, term: &str, now: i64) -> Result<()> {
    let term = terminal::clean(term);
    store.atomically(&mut || {
        let mut entries = read(store, &term, now)?;
        entries.retain(|(l, _, _)| *l != term);
        write(store, entries, now)
    })
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
