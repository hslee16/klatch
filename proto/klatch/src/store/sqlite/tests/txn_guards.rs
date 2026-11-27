//! The store transaction guards (#1676, #1678, #1677): a panic in `snapshot`, a ROLLBACK that fails, and the audit
//! flush accounting (rows not attempts, a counted drop past the cap, a failed row queued wherever it was made).

use super::*;
use crate::store::test_dir::temp_db;
use std::panic::{AssertUnwindSafe, catch_unwind};

const ACTION: &str = "demo-refused";
const NO_AUDIT: &str = "CREATE TRIGGER no_audit BEFORE INSERT ON admin_audit
    BEGIN SELECT RAISE(ABORT, 'full'); END;";

fn audit_rows(s: &SqliteStore) -> i64 {
    s.conn
        .query_row("SELECT count(*) FROM admin_audit", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn a_panic_inside_snapshot_closes_the_read_transaction() {
    let path = temp_db("snap-panic");
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let _ = s.snapshot(|| -> crate::domain::Result<()> { panic!("boom") });
    }));
    assert!(outcome.is_err());
    assert!(s.conn.is_autocommit(), "no transaction is left open");
    s.atomically(&mut || s.policy_set("label:after", Some("1"), 2))
        .unwrap();
    let other = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    assert_eq!(
        other.policy_get("label:after").unwrap().as_deref(),
        Some("1"),
        "the write after the panic committed"
    );
    other.policy_set("label:other", Some("1"), 3).unwrap();
}

#[test]
fn a_failed_rollback_is_counted_and_the_next_write_retries_it() {
    let s = SqliteStore::open_in_memory().unwrap();
    s.fail_rollbacks.set(1);
    let r = s.immediate(|| -> crate::domain::Result<()> { Err(Error::Backend("x".into())) });
    assert!(r.is_err());
    assert_eq!(s.rollback_errors(), 1);
    assert!(
        !s.conn.is_autocommit(),
        "the dead transaction is still open"
    );
    // the next write does not join it: it closes it first, then runs and commits
    s.policy_set("label:next", Some("1"), 1).unwrap();
    assert!(s.conn.is_autocommit());
    assert_eq!(s.policy_get("label:next").unwrap().as_deref(), Some("1"));
}

#[test]
fn a_write_refuses_to_join_a_dead_transaction_it_cannot_close() {
    let s = SqliteStore::open_in_memory().unwrap();
    s.fail_rollbacks.set(2);
    let _ = s.immediate(|| -> crate::domain::Result<()> { Err(Error::Backend("x".into())) });
    let mut ran = false;
    let r = s.immediate(|| {
        ran = true;
        Ok(())
    });
    assert!(r.is_err(), "refused, not joined");
    assert!(!ran, "the closure never ran inside the dead transaction");
    assert_eq!(s.rollback_errors(), 2);
}

#[test]
fn a_row_that_fails_many_flushes_is_one_failure() {
    let s = SqliteStore::open_in_memory().unwrap();
    s.conn.execute_batch(NO_AUDIT).unwrap();
    for _ in 0..3 {
        s.immediate(|| {
            if s.deferred_audit.borrow().is_empty() {
                s.audit_refusal(ACTION, "k", 1);
            }
            Ok(())
        })
        .unwrap();
    }
    assert_eq!(s.audit_flush_errors(), 1, "one row, three failed flushes");
    assert_eq!(s.audit_rows_dropped(), 0);
}

#[test]
fn a_failed_row_outside_a_transaction_is_queued_and_written_later() {
    let s = SqliteStore::open_in_memory().unwrap();
    s.conn.execute_batch(NO_AUDIT).unwrap();
    s.audit_refusal(ACTION, "k", 1);
    assert_eq!(s.audit_flush_errors(), 1);
    s.immediate(|| Ok(())).unwrap();
    assert_eq!(
        s.audit_flush_errors(),
        1,
        "a second failed flush is not a second failure"
    );
    s.conn.execute_batch("DROP TRIGGER no_audit").unwrap();
    s.immediate(|| Ok(())).unwrap();
    assert_eq!(audit_rows(&s), 1, "the row was retried");
    assert_eq!(s.audit_flush_errors(), 1, "and not counted twice");
}

#[test]
fn the_retry_queue_keeps_the_newest_256_rows_and_counts_the_rest_as_dropped() {
    let s = SqliteStore::open_in_memory().unwrap();
    s.conn.execute_batch(NO_AUDIT).unwrap();
    s.immediate(|| {
        for i in 0..300 {
            s.audit_refusal(ACTION, "k", i);
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(s.deferred_audit.borrow().len(), 256);
    assert_eq!(s.audit_rows_dropped(), 44);
    assert_eq!(s.audit_flush_errors(), 300, "every row failed once");
    s.conn.execute_batch("DROP TRIGGER no_audit").unwrap();
    s.immediate(|| Ok(())).unwrap();
    assert_eq!(audit_rows(&s), 256);
    let oldest: i64 = s
        .conn
        .query_row("SELECT min(ts) FROM admin_audit", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        oldest, 44,
        "the 44 oldest (ts 0..=43) were the ones dropped"
    );
}

fn queued(s: &SqliteStore) -> Vec<(String, i64)> {
    s.deferred_audit
        .borrow()
        .iter()
        .map(|r| (r.1.clone(), r.2))
        .collect()
}

#[test]
fn a_new_failed_row_outside_a_transaction_goes_behind_the_queued_ones() {
    let s = SqliteStore::open_in_memory().unwrap();
    s.conn.execute_batch(NO_AUDIT).unwrap();
    s.immediate(|| {
        s.audit_refusal(ACTION, "a", 1);
        Ok(())
    })
    .unwrap();
    s.audit_refusal(ACTION, "b", 2);
    assert_eq!(queued(&s), vec![("a".into(), 1), ("b".into(), 2)]);
}

#[test]
fn a_full_queue_drops_the_oldest_row_never_the_new_one() {
    let s = SqliteStore::open_in_memory().unwrap();
    s.conn.execute_batch(NO_AUDIT).unwrap();
    s.immediate(|| {
        for i in 0..256 {
            s.audit_refusal(ACTION, "k", i);
        }
        Ok(())
    })
    .unwrap();
    s.audit_refusal(ACTION, "new", 256);
    let q = queued(&s);
    assert_eq!(q.len(), 256);
    assert_eq!(q.first().unwrap().1, 1, "the oldest (ts 0) was dropped");
    assert_eq!(q.last().unwrap(), &("new".to_string(), 256));
    assert_eq!(s.audit_rows_dropped(), 1);
}

#[test]
fn a_snapshot_and_a_plain_read_close_a_dead_transaction_first() {
    let s = SqliteStore::open_in_memory().unwrap();
    s.fail_rollbacks.set(1);
    let _ = s.immediate(|| -> crate::domain::Result<()> {
        s.policy_set("label:dead", Some("1"), 1).unwrap();
        Err(Error::Backend("x".into()))
    });
    assert!(!s.conn.is_autocommit());
    assert_eq!(
        s.policy_get("label:dead").unwrap(),
        None,
        "no uncommitted row is seen"
    );
    assert!(s.conn.is_autocommit());
    s.fail_rollbacks.set(1);
    let _ = s.immediate(|| -> crate::domain::Result<()> { Err(Error::Backend("x".into())) });
    assert_eq!(s.snapshot(|| Ok(7)).unwrap(), 7);
}

#[test]
fn a_snapshot_and_a_read_refuse_a_dead_transaction_that_will_not_close() {
    let s = SqliteStore::open_in_memory().unwrap();
    s.fail_rollbacks.set(3);
    let _ = s.immediate(|| -> crate::domain::Result<()> { Err(Error::Backend("x".into())) });
    let mut ran = false;
    assert!(
        s.snapshot(|| {
            ran = true;
            Ok(())
        })
        .is_err()
    );
    assert!(!ran);
    assert!(s.policy_get("label:x").is_err(), "a read refuses too");
}

#[test]
fn a_write_inside_a_snapshot_is_an_error_not_a_silent_rollback() {
    let s = SqliteStore::open_in_memory().unwrap();
    let r = s.snapshot(|| s.policy_set("label:w", Some("1"), 1));
    assert!(r.is_err(), "{r:?}");
    assert!(s.conn.is_autocommit());
    assert_eq!(s.policy_get("label:w").unwrap(), None);
}
