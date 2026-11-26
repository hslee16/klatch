//! The write transaction of `immediate` closes however it ends (#1591, #1592): a panic inside it, a COMMIT that
//! fails with the transaction still open, and a failed flush of the queued audit rows are each reported or kept,
//! never silent.

use super::*;
use crate::store::test_dir::temp_db;
use std::panic::{AssertUnwindSafe, catch_unwind};

const ACTION: &str = "demo-refused";

fn actions(s: &SqliteStore) -> Vec<String> {
    s.audit_log(50)
        .unwrap()
        .into_iter()
        .map(|a| a.action)
        .collect()
}

#[test]
fn a_panic_inside_the_transaction_rolls_it_back_and_keeps_the_queued_audit_row() {
    let path = temp_db("imm-panic");
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let _ = s.immediate(|| -> crate::domain::Result<()> {
            s.audit_refusal(ACTION, "k", 1);
            panic!("boom");
        });
    }));
    assert!(outcome.is_err());
    assert!(s.conn.is_autocommit(), "no transaction is left open");
    assert_eq!(actions(&s), vec![ACTION.to_string()], "the refusal is kept");
    // a later write commits and a second connection can write
    s.atomically(&mut || s.policy_set("label:after", Some("1"), 2))
        .unwrap();
    let other = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    assert_eq!(
        other.policy_get("label:after").unwrap().as_deref(),
        Some("1")
    );
    other.policy_set("label:other", Some("1"), 3).unwrap();
}

#[test]
fn a_commit_that_fails_with_the_transaction_open_still_keeps_the_audit_row() {
    let s = SqliteStore::open_in_memory().unwrap();
    s.conn
        .execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE par(id INTEGER PRIMARY KEY);
             CREATE TABLE chi(p INTEGER REFERENCES par(id) DEFERRABLE INITIALLY DEFERRED);",
        )
        .unwrap();
    let r = s.immediate(|| {
        s.conn.execute_batch("INSERT INTO chi VALUES(99)").unwrap();
        s.audit_refusal(ACTION, "k", 1);
        Ok(())
    });
    assert!(r.is_err(), "the COMMIT failed and is reported");
    assert!(s.conn.is_autocommit(), "the failed transaction is closed");
    assert_eq!(actions(&s), vec![ACTION.to_string()], "the row survived");
    let n: i64 = s
        .conn
        .query_row("SELECT count(*) FROM chi", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "the failed transaction's own rows are gone");
}

#[test]
fn a_failed_flush_is_counted_not_silent() {
    let s = SqliteStore::open_in_memory().unwrap();
    s.conn
        .execute_batch(
            "CREATE TRIGGER no_audit BEFORE INSERT ON admin_audit
             BEGIN SELECT RAISE(ABORT, 'full'); END;",
        )
        .unwrap();
    assert_eq!(s.audit_flush_errors(), 0);
    s.immediate(|| {
        s.audit_refusal(ACTION, "k", 1);
        Ok(())
    })
    .unwrap();
    assert_eq!(s.audit_flush_errors(), 1);
    // an immediate (no transaction open) refusal that cannot be written is counted too
    s.audit_refusal(ACTION, "k2", 2);
    assert_eq!(s.audit_flush_errors(), 2);
}

#[test]
fn rows_whose_flush_failed_are_written_by_the_next_flush() {
    let s = SqliteStore::open_in_memory().unwrap();
    s.conn
        .execute_batch(
            "CREATE TRIGGER no_audit BEFORE INSERT ON admin_audit
             BEGIN SELECT RAISE(ABORT, 'full'); END;",
        )
        .unwrap();
    s.immediate(|| {
        s.audit_refusal(ACTION, "k", 1);
        Ok(())
    })
    .unwrap();
    s.conn.execute_batch("DROP TRIGGER no_audit").unwrap();
    s.immediate(|| Ok(())).unwrap();
    assert_eq!(actions(&s), vec![ACTION.to_string()]);
}
