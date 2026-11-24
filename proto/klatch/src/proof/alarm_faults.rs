//! What the alarm does when the store misbehaves (#1310): it never changes what a proof answers, never loses the
//! failure row for its sake, and two failures racing at the line write one alarm row, not two or none.

use super::alarm::{self, ACTION, CEILING};
use super::tests::{PASS, Typed, at, with_pass};
use super::*;
use crate::domain::{AuditRecord, Error, Result};
use crate::store::Census;
use crate::store::SqliteStore;
use crate::store::areas::PolicyStore;
use std::cell::{Cell, RefCell};
use std::time::Duration;

const CEIL: usize = CEILING as usize;

fn rows_of(s: &dyn PolicyStore, action: &str) -> Vec<AuditRecord> {
    s.audit_log(10_000)
        .unwrap()
        .into_iter()
        .filter(|a| a.action == action)
        .collect()
}

/// A store that behaves as the real one except that the census fails (a damaged index, a busy read, a store that
/// has no census at all).
struct NoCensus(SqliteStore, Cell<usize>);

impl PolicyStore for NoCensus {
    fn record_audit(&self, e: &crate::store::NewAudit, n: i64) -> Result<()> {
        self.1.set(self.1.get() + 1);
        self.0.record_audit(e, n)
    }
    fn audit_log(&self, l: usize) -> Result<Vec<AuditRecord>> {
        self.0.audit_log(l)
    }
    fn last_audit_ts(&self, a: &str, t: &str, s: i64) -> Result<Option<i64>> {
        self.0.last_audit_ts(a, t, s)
    }
    fn audit_census(&self, _: &Census) -> Result<(u64, u64)> {
        Err(Error::Backend("the census is broken".into()))
    }
    fn policy_get(&self, k: &str) -> Result<Option<String>> {
        self.0.policy_get(k)
    }
    fn policy_set(&self, k: &str, v: Option<&str>, n: i64) -> Result<()> {
        self.0.policy_set(k, v, n)
    }
    fn atomically(&self, act: &mut dyn FnMut() -> Result<()>) -> Result<()> {
        self.0.atomically(act)
    }
}

#[test]
fn a_census_that_fails_never_changes_what_a_proof_answers_or_loses_a_failure_row() {
    let s = NoCensus(SqliteStore::open_in_memory().unwrap(), Cell::new(0));
    with_pass(&s.0);
    for i in 0..60 {
        let tty: &'static str = Box::leak(format!("ttys{:03}/s{i}", i / 3).into_boxed_str());
        let e = prove(
            &s,
            &mut Typed::new(&["nope nope nope"]).on(tty),
            at(1000),
            &Unlock::new(),
            "a test",
        )
        .unwrap_err();
        assert_eq!(e, ProofError::Wrong, "failure {i}");
    }
    assert_eq!(rows_of(&s, alarm::FAILED).len(), 60);
    // one write each: a census error does not roll the failure back to write it a second time
    assert_eq!(s.1.get(), 60);
    assert!(rows_of(&s, ACTION).is_empty());
    let mut fresh = Typed::new(&[PASS]).on("ttys999/s999");
    prove(&s, &mut fresh, at(1001), &Unlock::new(), "a test").unwrap();
}

/// A store whose transaction cannot start (another process holds the write lock past the wait).
struct NoTransaction(SqliteStore);

impl PolicyStore for NoTransaction {
    fn record_audit(&self, e: &crate::store::NewAudit, n: i64) -> Result<()> {
        self.0.record_audit(e, n)
    }
    fn audit_log(&self, l: usize) -> Result<Vec<AuditRecord>> {
        self.0.audit_log(l)
    }
    fn audit_census(&self, q: &Census) -> Result<(u64, u64)> {
        self.0.audit_census(q)
    }
    fn atomically(&self, _: &mut dyn FnMut() -> Result<()>) -> Result<()> {
        Err(Error::Busy("database is locked".into()))
    }
}

#[test]
fn when_the_transaction_cannot_start_the_failure_row_is_still_written() {
    let s = NoTransaction(SqliteStore::open_in_memory().unwrap());
    alarm::failed(&s, "", "failures=1 terminal=ttys001/s1", 1000);
    let rows = rows_of(&s, alarm::FAILED);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].detail, "failures=1 terminal=ttys001/s1");
    assert!(rows_of(&s, ACTION).is_empty());
}

/// A store that, between the insert of a failure and the census that follows it, lets a second process (its own
/// connection to the same file) fail once too. With the insert and the census in one write transaction the second
/// process waits for the first to finish; without it the second slips in between and both see the line crossed.
struct Racing {
    inner: SqliteStore,
    path: std::path::PathBuf,
    other: RefCell<Option<std::thread::JoinHandle<()>>>,
}

impl PolicyStore for Racing {
    fn record_audit(&self, e: &crate::store::NewAudit, n: i64) -> Result<()> {
        self.inner.record_audit(e, n)
    }
    fn audit_log(&self, l: usize) -> Result<Vec<AuditRecord>> {
        self.inner.audit_log(l)
    }
    fn audit_census(&self, q: &Census) -> Result<(u64, u64)> {
        if self.other.borrow().is_none() {
            let path = self.path.clone();
            *self.other.borrow_mut() = Some(std::thread::spawn(move || {
                let second = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
                alarm::failed(&second, "", "failures=1 terminal=ttys-second/s2", 1000);
            }));
            // long enough for an unguarded second writer to finish its insert and its own census
            std::thread::sleep(Duration::from_millis(400));
        }
        self.inner.audit_census(q)
    }
    fn atomically(&self, act: &mut dyn FnMut() -> Result<()>) -> Result<()> {
        self.inner.atomically(act)
    }
}

#[test]
fn two_failures_racing_at_the_line_write_exactly_one_alarm_row() {
    let dir = crate::store::test_dir::TestDir::new("alarm-race");
    let path = dir.join("race.db");
    let key = crate::store::insecure_test_key();
    let first = SqliteStore::open(&path, &key).unwrap();
    for i in 0..CEIL - 2 {
        first
            .record_audit(
                &crate::store::NewAudit::new(
                    alarm::FAILED,
                    "",
                    &format!("failures=1 terminal=t{i}"),
                ),
                1000,
            )
            .unwrap();
    }
    let s = Racing {
        inner: first,
        path,
        other: RefCell::new(None),
    };
    // this failure makes CEIL - 1; the second process's makes CEIL; they must not both see the line crossed
    alarm::failed(&s, "", "failures=1 terminal=ttys-first/s1", 1000);
    s.other.borrow_mut().take().unwrap().join().unwrap();
    assert_eq!(rows_of(&s, alarm::FAILED).len(), CEIL);
    let rows = rows_of(&s, ACTION);
    assert_eq!(rows.len(), 1, "{rows:?}");
}
