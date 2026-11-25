//! #543: a deadlock refusal must mean a real cycle in the wait-for graph. Each case here is a refusal that closes no
//! cycle (a wait on another resource, an old wait, another room or scope, a satisfied wait, a departed waiter) or a
//! cycle that is real; the first group must never be flagged, the last group must always be.

use super::*;
use crate::domain::Error;

const R: &str = "r";

fn take_in(
    s: &SqliteStore,
    room: &str,
    who: &str,
    path: &str,
    now: i64,
) -> crate::domain::Result<()> {
    s.claim(&ClaimRequest::new((room, who), path, 1_000, ""), now)
}

fn take(s: &SqliteStore, who: &str, path: &str, now: i64) -> crate::domain::Result<()> {
    take_in(s, R, who, path, now)
}

fn scoped(
    s: &SqliteStore,
    who: &str,
    scope: &str,
    path: &str,
    now: i64,
) -> crate::domain::Result<()> {
    s.claim(
        &ClaimRequest::new((R, who), path, 1_000, "").scoped(scope),
        now,
    )
}

fn passive(s: &SqliteStore, who: &str, path: &str, now: i64) {
    s.take_passive_claim(&ClaimRequest::new((R, who), path, 1_000, "passive"), now)
        .unwrap();
}

/// The attempt is refused; whether the refusal says deadlock.
fn refused(r: crate::domain::Result<()>) -> bool {
    match r {
        Err(Error::Conflict { deadlock, .. }) => deadlock,
        other => panic!("not a conflict: {other:?}"),
    }
}

#[test]
fn a_design_item_refusal_with_a_holder_that_waits_on_nobody_is_no_deadlock() {
    let s = store();
    take(&s, "dana", "design-item:5", 0).unwrap();
    take(&s, "alice", "f1", 1).unwrap();
    take(&s, "alice", "f2", 1).unwrap();
    take(&s, "bob", "g", 1).unwrap();
    assert!(!refused(take(&s, "alice", "g", 2)), "alice waits on g");
    assert!(!refused(take(&s, "bob", "design-item:5", 3)));
}

#[test]
fn a_wait_on_x_does_not_make_a_claim_on_y_a_deadlock() {
    // bob holds design-item:7 and waits for x (carol's); alice holds files and asks for design-item:7. bob waits for
    // carol, not for alice, so there is no cycle however many files alice holds.
    let s = store();
    take(&s, "carol", "x", 0).unwrap();
    take(&s, "bob", "design-item:7", 0).unwrap();
    take(&s, "alice", "f1", 0).unwrap();
    take(&s, "alice", "f2", 0).unwrap();
    take(&s, "alice", "f3/deep", 0).unwrap();
    assert!(!refused(take(&s, "bob", "x", 1)));
    assert!(!refused(take(&s, "alice", "design-item:7", 2)));
}

#[test]
fn design_item_numbers_that_share_a_digit_prefix_do_not_overlap() {
    let s = store();
    take(&s, "bob", "design-item:1", 0).unwrap();
    take(&s, "alice", "f", 0).unwrap();
    take(&s, "alice", "design-item:10", 1).unwrap();
    take(&s, "carol", "design-item:100", 1).unwrap();
    assert!(!refused(take(&s, "bob", "f", 2)));
    take(&s, "bob", "design-item:11", 3).unwrap();
}

#[test]
fn a_wait_on_a_sibling_path_is_not_an_edge_to_the_holder_of_another_sibling() {
    // bob waits for dir/a (carol's); alice holds dir/b and asks for bob's design-item:9. dir/b does not overlap dir/a.
    let s = store();
    take(&s, "carol", "dir/a", 0).unwrap();
    take(&s, "alice", "dir/b", 0).unwrap();
    take(&s, "bob", "design-item:9", 0).unwrap();
    assert!(!refused(take(&s, "bob", "dir/a", 1)));
    assert!(!refused(take(&s, "alice", "design-item:9", 2)));
}

#[test]
fn an_old_wait_is_not_an_edge() {
    let s = store();
    take(&s, "alice", "f", 0).unwrap();
    take(&s, "bob", "design-item:3", 0).unwrap();
    assert!(!refused(take(&s, "bob", "f", 1)), "bob waits on alice");
    // far past the wait window, before anyone's lease (1000s) ends
    assert!(!refused(take(&s, "alice", "design-item:3", 900)));
}

#[test]
fn a_wait_in_another_room_is_not_an_edge() {
    let s = store();
    take(&s, "alice", "f", 0).unwrap();
    take_in(&s, "other", "bob", "design-item:3", 0).unwrap();
    take_in(&s, "other", "alice", "f", 0).unwrap();
    take_in(&s, "other", "carol", "g", 0).unwrap();
    assert!(!refused(take_in(&s, "other", "bob", "g", 1)));
    // in room r bob holds nothing alice needs and has no wait
    take(&s, "bob", "design-item:3", 2).unwrap();
    assert!(!refused(take(&s, "alice", "design-item:3", 3)));
}

#[test]
fn a_wait_in_a_scope_that_does_not_collide_is_not_an_edge() {
    // bob (s2) holds design-item:3 and waits for g (dana's, s2). alice holds g/x in s1: it overlaps bob's wait but
    // does not collide with s2, so it does not block bob, and alice asking for design-item:3 in s2 is no cycle.
    let s = store();
    scoped(&s, "dana", "s2", "g", 0).unwrap();
    scoped(&s, "alice", "s1", "g/x", 0).unwrap();
    scoped(&s, "bob", "s2", "design-item:3", 0).unwrap();
    assert!(
        !refused(scoped(&s, "bob", "s2", "g", 1)),
        "bob waits on dana"
    );
    assert!(!refused(scoped(&s, "alice", "s2", "design-item:3", 2)));
}

#[test]
fn a_wait_that_was_satisfied_is_not_an_edge() {
    let s = store();
    take(&s, "alice", "f", 0).unwrap();
    take(&s, "bob", "design-item:3", 0).unwrap();
    assert!(!refused(take(&s, "bob", "f", 1)));
    s.release(R, "alice", "f").unwrap();
    take(&s, "bob", "f", 2).unwrap();
    take(&s, "alice", "g", 3).unwrap();
    assert!(!refused(take(&s, "alice", "design-item:3", 4)));
}

#[test]
fn a_waiter_that_released_what_the_asker_needs_is_not_a_cycle() {
    // bob waits on alice's f, and alice asks for bob's design-item:3; bob let go of it first.
    let s = store();
    take(&s, "alice", "f", 0).unwrap();
    take(&s, "bob", "design-item:3", 0).unwrap();
    assert!(!refused(take(&s, "bob", "f", 1)));
    s.release(R, "bob", "design-item:3").unwrap();
    take(&s, "carol", "design-item:3", 2).unwrap();
    assert!(
        !refused(take(&s, "alice", "design-item:3", 3)),
        "carol waits on nobody"
    );
}

#[test]
fn a_waiter_removed_from_every_room_leaves_no_edge() {
    let s = store();
    take(&s, "alice", "f", 0).unwrap();
    take(&s, "bob", "design-item:3", 0).unwrap();
    assert!(!refused(take(&s, "bob", "f", 1)));
    s.release(R, "bob", "design-item:3").unwrap();
    s.prune_members(1, 100_000).unwrap();
    take(&s, "bob", "design-item:3", 100_001).unwrap_or(());
    let waits: i64 = s
        .conn
        .query_row(
            "SELECT COUNT(*) FROM claim_waits WHERE member='bob'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(waits, 0);
}

#[test]
fn shared_claims_that_do_not_wait_on_the_asker_are_no_deadlock() {
    let s = store();
    scoped(&s, "alice", "s1", "f", 0).unwrap();
    scoped(&s, "bob", "*", "design-item:3", 0).unwrap();
    scoped(&s, "dana", "s1", "g", 0).unwrap();
    assert!(
        !refused(scoped(&s, "bob", "s1", "g", 1)),
        "bob waits on dana"
    );
    assert!(!refused(scoped(&s, "alice", "s1", "design-item:3", 2)));
}

#[test]
fn a_wait_ended_by_a_passive_claim_is_not_an_edge_after_the_claim_is_released() {
    // alice waits for p (bob's); bob lets go; alice's hook takes p passively; alice then releases p and carol takes
    // it. alice no longer wants p, so carol asking for alice's r is no deadlock.
    let s = store();
    take(&s, "bob", "p", 0).unwrap();
    take(&s, "alice", "r", 0).unwrap();
    assert!(!refused(take(&s, "alice", "p", 1)));
    s.release(R, "bob", "p").unwrap();
    passive(&s, "alice", "p", 2);
    s.release(R, "alice", "p").unwrap();
    take(&s, "carol", "p", 3).unwrap();
    assert!(!refused(take(&s, "carol", "r", 4)));
}

#[test]
fn a_two_way_cycle_through_a_design_item_and_a_file_is_a_deadlock() {
    let s = store();
    take(&s, "alice", "f", 0).unwrap();
    take(&s, "bob", "design-item:3", 0).unwrap();
    assert!(!refused(take(&s, "bob", "f", 1)));
    assert!(refused(take(&s, "alice", "design-item:3", 2)));
}

#[test]
fn a_three_way_cycle_with_many_other_claims_is_a_deadlock() {
    let s = store();
    take(&s, "a", "design-item:1", 0).unwrap();
    take(&s, "b", "f", 0).unwrap();
    take(&s, "c", "g/sub", 0).unwrap();
    for n in 0..5 {
        take(&s, "a", &format!("noise{n}"), 0).unwrap();
    }
    assert!(!refused(take(&s, "b", "design-item:1", 1)));
    assert!(!refused(take(&s, "c", "f", 2)));
    assert!(refused(take(&s, "a", "g", 3)));
}
