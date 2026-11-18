//! The cycles of claim waits that stand now, read without telling anyone (#271).

use super::*;

fn take(s: &SqliteStore, who: &str, path: &str, now: i64) -> crate::domain::Result<()> {
    s.claim(&ClaimRequest::new(("r", who), path, 1_000, ""), now)
}

/// alice holds p and bob holds q; alice is refused q and bob is refused p: each waits for the other.
fn two_way(s: &SqliteStore) {
    take(s, "alice", "p", 0).unwrap();
    take(s, "bob", "q", 0).unwrap();
    assert!(take(s, "alice", "q", 1).is_err());
    assert!(take(s, "bob", "p", 2).is_err());
}

#[test]
fn a_standing_two_way_wait_is_one_cycle_and_reading_it_changes_nothing() {
    let s = store();
    two_way(&s);
    let want = vec![vec!["alice".to_string(), "bob".to_string()]];
    assert_eq!(s.wait_cycles("r", 3).unwrap(), want);
    assert_eq!(s.wait_cycles("r", 3).unwrap(), want, "read only, repeatable");
}

#[test]
fn a_plain_wait_with_no_way_back_is_not_a_cycle() {
    let s = store();
    take(&s, "alice", "p", 0).unwrap();
    assert!(take(&s, "bob", "p", 1).is_err());
    assert!(s.wait_cycles("r", 2).unwrap().is_empty());
}

#[test]
fn a_cycle_ends_when_a_wait_is_satisfied_or_older_than_the_window() {
    let s = store();
    two_way(&s);
    s.release("r", "bob", "q").unwrap();
    take(&s, "alice", "q", 4).unwrap();
    assert!(s.wait_cycles("r", 5).unwrap().is_empty(), "satisfied");
    let s = store();
    two_way(&s);
    assert!(s.wait_cycles("r", 1_000).unwrap().is_empty(), "old waits do not count");
}

#[test]
fn a_cycle_of_three_lists_all_three_and_another_room_is_separate() {
    let s = store();
    take(&s, "a", "x", 0).unwrap();
    take(&s, "b", "y", 0).unwrap();
    take(&s, "c", "z", 0).unwrap();
    assert!(take(&s, "a", "y", 1).is_err());
    assert!(take(&s, "b", "z", 2).is_err());
    assert!(take(&s, "c", "x", 3).is_err());
    let want = vec![vec!["a".to_string(), "b".to_string(), "c".to_string()]];
    assert_eq!(s.wait_cycles("r", 4).unwrap(), want);
    assert!(s.wait_cycles("other", 4).unwrap().is_empty());
}
