//! The summary row of a throttled minute (#1709) when its write fails: the count is kept for the next flush.

use super::*;
use crate::store::PolicyStore;

fn summary_details(s: &SqliteStore) -> Vec<String> {
    s.audit_log(1000)
        .unwrap()
        .into_iter()
        .filter(|a| a.action == "policy-refusals-throttled")
        .map(|a| a.detail)
        .collect()
}

#[test]
fn a_failed_summary_write_keeps_its_count_for_the_next_flush() {
    let s = SqliteStore::open_in_memory().unwrap();
    for i in 0..8 {
        assert!(
            s.policy_set(&format!("admin-unlock-j{i}"), Some("x"), 100)
                .is_err()
        );
    }
    s.fail_summaries.set(2);
    for i in 0..2 {
        assert!(
            s.policy_set(&format!("admin-unlock-k{i}"), Some("x"), 101)
                .is_err()
        );
    }
    assert!(summary_details(&s).is_empty(), "both writes failed");
    assert!(s.policy_set("admin-unlock-k9", Some("x"), 102).is_err());
    assert_eq!(summary_details(&s), ["3 refusals throttled"]);
}
