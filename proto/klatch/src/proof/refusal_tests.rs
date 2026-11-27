//! The audit rows of refused writes to the admin passphrase rows are bounded (#1588): anyone who holds the store can
//! refuse a write in a loop, and the loop must not push real rows out of the audit.

use super::tests::store;
use crate::store::PolicyStore;

fn all_refusals(s: &impl PolicyStore) -> usize {
    s.audit_log(100_000)
        .unwrap()
        .iter()
        .filter(|a| a.action == "policy-reserved-refused")
        .count()
}

fn refusals(s: &impl PolicyStore, key: &str) -> usize {
    s.audit_log(1000)
        .unwrap()
        .iter()
        .filter(|a| a.action == "policy-reserved-refused" && a.target == key)
        .count()
}

#[test]
fn five_hundred_refused_writes_leave_one_audit_row_a_minute_per_key() {
    let s = store();
    for i in 0..500 {
        assert!(
            s.policy_set("admin-passphrase", Some("x"), 1000 + i % 30)
                .is_err()
        );
    }
    assert_eq!(refusals(&s, "admin-passphrase"), 1);
    // another key is audited on its own, and the same key again once the minute has passed
    assert!(s.policy_set("admin-unlock-secs", Some("x"), 1001).is_err());
    assert_eq!(refusals(&s, "admin-unlock-secs"), 1);
    assert!(s.policy_set("admin-passphrase", Some("x"), 1100).is_err());
    assert_eq!(refusals(&s, "admin-passphrase"), 2);
}

#[test]
fn every_admin_key_family_is_refused_to_the_generic_writer() {
    let s = store();
    for k in [
        "admin-passphrase",
        "admin-passphrase-fails",
        "admin-unlock-secs",
        "admin-unlock-until",
        "admin-unlock-anything",
    ] {
        assert!(s.policy_set(k, Some("x"), 1).is_err(), "{k}");
    }
    assert!(s.policy_set("admin:note", Some("x"), 1).is_ok());
    assert!(
        s.policy_set("label:admin-passphrase-room", Some("x"), 1)
            .is_ok()
    );
}

#[test]
fn a_thousand_refusals_inside_one_transaction_queue_one_row_per_key_per_minute() {
    let s = store();
    s.atomically(&mut || {
        for i in 0..1000 {
            assert!(
                s.policy_set("admin-passphrase", Some("x"), 2000 + i % 30)
                    .is_err()
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(refusals(&s, "admin-passphrase"), 1);
    assert_eq!(
        s.audit_rows_dropped(),
        999,
        "the throttled refusals are counted"
    );
}

#[test]
fn distinct_admin_unlock_keys_cannot_get_past_the_throttle() {
    let s = store();
    for i in 0..300 {
        assert!(
            s.policy_set(&format!("admin-unlock-{i}"), Some("x"), 3000)
                .is_err()
        );
    }
    assert_eq!(
        all_refusals(&s),
        8,
        "a family is capped per minute, whatever the key name"
    );
    assert_eq!(s.audit_rows_dropped(), 292);
    // the same inside a transaction, and again after the minute
    s.atomically(&mut || {
        for i in 0..300 {
            assert!(
                s.policy_set(&format!("admin-unlock-t{i}"), Some("x"), 3001)
                    .is_err()
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(all_refusals(&s), 8);
    assert!(s.policy_set("admin-unlock-late", Some("x"), 3100).is_err());
    assert_eq!(all_refusals(&s), 9);
}

#[test]
fn a_row_stamped_in_the_future_does_not_silence_refusal_audits() {
    let s = store();
    assert!(s.policy_set("admin-passphrase", Some("x"), 10_000).is_err());
    // the clock stepped back an hour: the row at 10_000 is in the future
    assert!(s.policy_set("admin-passphrase", Some("x"), 6_400).is_err());
    assert_eq!(refusals(&s, "admin-passphrase"), 2);
}

#[test]
fn a_future_stamped_row_queued_in_one_transaction_does_not_silence_the_next_either() {
    let s = store();
    s.atomically(&mut || {
        assert!(s.policy_set("admin-passphrase", Some("x"), 10_000).is_err());
        assert!(s.policy_set("admin-passphrase", Some("x"), 6_400).is_err());
        Ok(())
    })
    .unwrap();
    assert_eq!(refusals(&s, "admin-passphrase"), 2);
}
