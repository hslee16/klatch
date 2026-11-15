//! Claim age (#547): when a claim was first held, kept across renewals, and reported to its owner once per milestone.

use super::*;
use crate::domain::Error;

/// Every moment is shifted by this, because a start of 0 means unknown.
const T0: i64 = 1_000;

fn ages(s: &SqliteStore, owner: &str, now: i64) -> Vec<(String, i64)> {
    s.claim_ages("r", owner, T0 + now)
        .unwrap()
        .into_iter()
        .map(|a| (a.path, a.held_secs))
        .collect()
}

fn held(s: &SqliteStore, owner: &str, path: &str, (ttl, at): (i64, i64)) {
    s.claim(&ClaimRequest::new(("r", owner), path, ttl, ""), T0 + at)
        .unwrap();
}

#[test]
fn a_claim_held_two_hours_is_reported_to_its_owner_once_per_milestone() {
    let s = store();
    held(&s, "alice", "a", (86_400, 0));
    assert!(ages(&s, "alice", 7_199).is_empty());
    assert_eq!(ages(&s, "alice", 7_200), vec![("a".to_string(), 7_200)]);
    assert!(ages(&s, "alice", 7_300).is_empty(), "told once");
    assert_eq!(ages(&s, "alice", 28_900), vec![("a".to_string(), 28_900)]);
    assert!(ages(&s, "alice", 29_000).is_empty());
}

#[test]
fn claiming_again_to_renew_keeps_the_age() {
    let s = store();
    held(&s, "alice", "a", (86_400, 0));
    held(&s, "alice", "a", (86_400, 7_000));
    assert_eq!(ages(&s, "alice", 7_300), vec![("a".to_string(), 7_300)]);
}

#[test]
fn activity_renewal_keeps_the_age() {
    let s = store();
    held(&s, "alice", "a", (600, 0));
    for t in (500..=7_300).step_by(500) {
        s.touch("r", "alice", T0 + t).unwrap();
    }
    assert_eq!(ages(&s, "alice", 7_300), vec![("a".to_string(), 7_300)]);
}

#[test]
fn a_claim_that_lapsed_and_was_taken_again_starts_a_new_age() {
    let s = store();
    held(&s, "alice", "a", (100, 0));
    held(&s, "alice", "a", (86_400, 5_000));
    assert!(ages(&s, "alice", 7_300).is_empty());
    assert_eq!(ages(&s, "alice", 12_200), vec![("a".to_string(), 7_200)]);
}

#[test]
fn an_ask_lease_and_a_claim_from_before_the_column_are_never_reported() {
    let s = store();
    held(&s, "alice", "ask:7", (86_400, 0));
    held(&s, "alice", "old", (86_400, 0));
    s.conn
        .execute("UPDATE claims SET since=0 WHERE path='old'", [])
        .unwrap();
    assert!(ages(&s, "alice", 30_000).is_empty());
}

#[test]
fn only_the_owner_is_told_and_only_for_its_own_room() {
    let s = store();
    held(&s, "alice", "a", (86_400, 0));
    assert!(ages(&s, "bob", 8_000).is_empty());
    assert!(
        s.claim_ages("other", "alice", T0 + 8_000)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_refusal_tells_how_long_the_holder_has_held_the_claim() {
    let s = store();
    held(&s, "alice", "a", (86_400, 0));
    match s.claim(&ClaimRequest::new(("r", "bob"), "a", 60, ""), T0 + 9_000) {
        Err(Error::Conflict { held_secs, .. }) => assert_eq!(held_secs, 9_000),
        other => panic!("{other:?}"),
    }
}
