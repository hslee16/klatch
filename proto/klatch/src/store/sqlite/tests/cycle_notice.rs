//! A cycle of waits completed by a claim being taken is told once to the member the new claim blocks (#648, DESIGN
//! section 32 item 82).

use super::*;
use crate::domain::{CycleNotice, Error};

fn take(s: &SqliteStore, who: &str, path: &str, now: i64) -> crate::domain::Result<()> {
    s.claim(&ClaimRequest::new(("r", who), path, 1_000, ""), now)
}

/// Carol holds p and alice holds q; alice is refused p, bob is refused q; carol lets go of p and bob takes it. Now
/// alice waits for bob (bob holds p) and bob waits for alice (alice holds q): a cycle that no refusal closed.
fn a_cycle_completed_by_a_take(s: &SqliteStore) {
    take(s, "carol", "p", 0).unwrap();
    take(s, "alice", "q", 0).unwrap();
    assert!(take(s, "alice", "p", 1).is_err());
    assert!(take(s, "bob", "q", 2).is_err());
    s.release("r", "carol", "p").unwrap();
    take(s, "bob", "p", 3).unwrap();
}

#[test]
fn a_cycle_completed_by_a_claim_being_taken_is_told_once_to_the_member_it_newly_blocks() {
    let s = store();
    a_cycle_completed_by_a_take(&s);
    let told = s.deadlock_notices("r", "alice", 4).unwrap();
    assert_eq!(
        told,
        [CycleNotice {
            path: "p".into(),
            holder: "bob".into()
        }]
    );
    assert!(
        s.deadlock_notices("r", "alice", 5).unwrap().is_empty(),
        "once"
    );
    assert!(
        s.deadlock_notices("r", "bob", 4).unwrap().is_empty(),
        "only the member the new claim blocks"
    );
}

#[test]
fn a_refusal_by_two_holders_is_a_deadlock_when_the_cycle_runs_through_the_second() {
    // Found by the model while checking #648: the refusal names the first holder, and a cycle through another holder
    // of an overlapping path was not flagged. Dave holds a/b (first), carol holds a/c and waits for c, alice holds c
    // and asks for a: dave and carol both refuse her, and carol waits for her.
    let s = store();
    take(&s, "dave", "a/b", 0).unwrap();
    take(&s, "carol", "a/c", 1).unwrap();
    take(&s, "alice", "c", 2).unwrap();
    assert!(take(&s, "carol", "c", 3).is_err());
    let refused = take(&s, "alice", "a", 4).unwrap_err();
    assert!(
        matches!(refused, Error::Conflict { ref owner, deadlock: true, .. } if owner == "dave"),
        "{refused:?}"
    );
}

#[test]
fn a_claim_taken_that_completes_no_cycle_tells_nobody() {
    let s = store();
    take(&s, "carol", "p", 0).unwrap();
    assert!(take(&s, "alice", "p", 1).is_err());
    s.release("r", "carol", "p").unwrap();
    take(&s, "bob", "p", 2).unwrap();
    assert!(s.deadlock_notices("r", "alice", 3).unwrap().is_empty());
}

#[test]
fn a_notice_for_a_wait_that_aged_out_is_not_given() {
    let s = store();
    a_cycle_completed_by_a_take(&s);
    let late = 3 + super::super::WAIT_WINDOW_SECS + 1;
    assert!(s.deadlock_notices("r", "alice", late).unwrap().is_empty());
}
