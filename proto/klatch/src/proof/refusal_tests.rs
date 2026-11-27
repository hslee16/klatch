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

const REAL_KEYS: [&str; 3] = [
    "admin-passphrase",
    "admin-passphrase-fails",
    "admin-unlock-secs",
];

fn junk_flood(s: &impl PolicyStore, family: &str, n: usize, ts: i64) {
    for i in 0..n {
        assert!(
            s.policy_set(&format!("{family}junk{i}"), Some("x"), ts)
                .is_err()
        );
    }
}

fn summaries(s: &impl PolicyStore) -> Vec<(i64, String, String)> {
    s.audit_log(100_000)
        .unwrap()
        .into_iter()
        .filter(|a| a.action == "policy-refusals-throttled")
        .map(|a| (a.ts, a.target, a.detail))
        .collect()
}

#[test]
fn a_junk_key_flood_cannot_hide_a_refusal_of_a_real_key() {
    let s = store();
    junk_flood(&s, "admin-passphrase-", 8, 4000);
    junk_flood(&s, "admin-unlock-", 8, 4000);
    for k in REAL_KEYS {
        assert!(s.policy_set(k, Some("x"), 4001).is_err());
        assert_eq!(refusals(&s, k), 1, "{k} keeps its own budget");
    }
    // and a real key's row does not spend the junk budget either
    let t = store();
    for k in REAL_KEYS {
        assert!(t.policy_set(k, Some("x"), 4000).is_err());
    }
    junk_flood(&t, "admin-passphrase-", 8, 4001);
    assert_eq!(all_refusals(&t), 3 + 8);
}

#[test]
fn a_real_key_is_still_one_row_a_minute() {
    let s = store();
    for i in 0..50 {
        assert!(
            s.policy_set("admin-unlock-secs", Some("x"), 5000 + i)
                .is_err()
        );
    }
    assert_eq!(refusals(&s, "admin-unlock-secs"), 1);
    assert!(s.policy_set("admin-unlock-secs", Some("x"), 5100).is_err());
    assert_eq!(refusals(&s, "admin-unlock-secs"), 2);
}

#[test]
fn a_throttled_minute_leaves_one_summary_row_with_the_count() {
    let s = store();
    junk_flood(&s, "admin-unlock-", 300, 6000);
    let rows = summaries(&s);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].2, "292 refusals throttled");
    assert_eq!(rows[0].1, "", "no key name, nothing the caller chose");
    // a later minute is its own row; a minute with no throttling has none
    junk_flood(&s, "admin-unlock-", 10, 6100);
    let rows = summaries(&s);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].2, "2 refusals throttled");
    assert_eq!(all_refusals(&s), 16);
}

#[test]
fn an_unthrottled_minute_has_no_summary() {
    let s = store();
    junk_flood(&s, "admin-unlock-", 8, 6500);
    assert!(summaries(&s).is_empty());
}

#[test]
fn the_summary_survives_a_flood_inside_one_transaction() {
    let s = store();
    s.atomically(&mut || {
        junk_flood(&s, "admin-passphrase-", 100, 7000);
        Ok(())
    })
    .unwrap();
    let rows = summaries(&s);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].2, "92 refusals throttled");
}

#[test]
fn real_admin_acts_are_never_throttled() {
    let s = store();
    junk_flood(&s, "admin-unlock-", 300, 8000);
    for action in ["administer", "admin_note", "judge", "renew-credential"] {
        for i in 0..40 {
            s.record_audit(
                &crate::store::NewAudit {
                    action,
                    target: "t",
                    detail: "",
                },
                8000 + i % 3,
            )
            .unwrap();
        }
    }
    for i in 0..40 {
        s.policy_set("admin:note", Some("x"), 8000 + i % 3).unwrap();
    }
    let log = s.audit_log(100_000).unwrap();
    for action in ["administer", "admin_note", "judge", "renew-credential"] {
        assert_eq!(log.iter().filter(|a| a.action == action).count(), 40);
    }
}
