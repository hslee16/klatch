//! The audit rows of refused writes to the admin passphrase rows are bounded (#1588): anyone who holds the store can
//! refuse a write in a loop, and the loop must not push real rows out of the audit.

use super::tests::store;
use crate::store::PolicyStore;

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
