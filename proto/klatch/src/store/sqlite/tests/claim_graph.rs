//! The blocker graph, the end of a wait, the claim age of an unknown start and the expiry boundaries (#726): each
//! condition of the cycle detection is the only false one in some case, so no `&&` can become `||` unseen.

use super::*;
use crate::domain::{CycleNotice, Error};
use crate::store::NewCredential;

fn take(s: &SqliteStore, who: &str, path: &str, now: i64) -> crate::domain::Result<()> {
    s.claim(&ClaimRequest::new(("r", who), path, 1_000, ""), now)
}

fn take_in(
    s: &SqliteStore,
    (who, scope): (&str, &str),
    path: &str,
    now: i64,
) -> crate::domain::Result<()> {
    s.claim(
        &ClaimRequest::new(("r", who), path, 1_000, "").scoped(scope),
        now,
    )
}

fn deadlock_flag(r: crate::domain::Result<()>) -> bool {
    match r {
        Err(Error::Conflict { deadlock, .. }) => deadlock,
        other => panic!("not a conflict: {other:?}"),
    }
}

#[test]
fn a_claim_nobody_waits_on_is_not_an_edge_of_the_wait_graph() {
    // bob holds p and waits for q (dave's). alice holds z, unrelated to anything bob waits for, and asks for p.
    // bob does not wait for alice, so alice's refusal closes no cycle.
    let s = store();
    take(&s, "dave", "q", 0).unwrap();
    take(&s, "bob", "p", 1).unwrap();
    take(&s, "alice", "z", 2).unwrap();
    assert!(take(&s, "bob", "q", 3).is_err());
    assert!(!deadlock_flag(take(&s, "alice", "p", 4)));
}

#[test]
fn a_claim_that_overlaps_the_wait_but_is_in_a_scope_that_does_not_collide_is_not_a_blocker() {
    // bob (scope s2) waits for q, held by dave in s2. alice holds q/x in s1: it overlaps bob's wait, but s1 and s2
    // do not collide, so it does not block bob; alice asking for bob's shared p does not close a cycle.
    let s = store();
    take_in(&s, ("dave", "s2"), "q", 0).unwrap();
    take_in(&s, ("bob", "*"), "p", 1).unwrap();
    take_in(&s, ("alice", "s1"), "q/x", 2).unwrap();
    assert!(take_in(&s, ("bob", "s2"), "q", 3).is_err());
    assert!(!deadlock_flag(take_in(&s, ("alice", "s1"), "p", 4)));
}

#[test]
fn a_real_cycle_is_still_flagged_when_the_holder_waits_on_the_asker() {
    let s = store();
    take(&s, "dave", "q", 0).unwrap();
    take(&s, "bob", "p", 1).unwrap();
    take(&s, "alice", "z", 2).unwrap();
    take(&s, "alice", "q/x", 2).unwrap_err();
    assert!(take(&s, "bob", "z", 3).is_err());
    assert!(deadlock_flag(take(&s, "alice", "p", 4)));
}

#[test]
fn taking_a_path_ends_the_wait_for_it_so_it_cannot_be_a_stale_edge_later() {
    // alice waits for p; carol lets go; alice takes p and lets go again; dave takes p and asks for alice's r. Alice
    // no longer waits for anything, so dave's refusal is no deadlock.
    let s = store();
    take(&s, "carol", "p", 0).unwrap();
    take(&s, "alice", "r", 1).unwrap();
    assert!(take(&s, "alice", "p", 2).is_err());
    s.release("r", "carol", "p").unwrap();
    take(&s, "alice", "p", 3).unwrap();
    s.release("r", "alice", "p").unwrap();
    take(&s, "dave", "p", 4).unwrap();
    assert!(!deadlock_flag(take(&s, "dave", "r", 5)));
}

#[test]
fn taking_a_path_ends_the_waits_for_paths_it_overlaps_and_keeps_the_others() {
    let s = store();
    take(&s, "carol", "p/x", 0).unwrap();
    take(&s, "erin", "z", 0).unwrap();
    take(&s, "alice", "r", 1).unwrap();
    assert!(take(&s, "alice", "p/x", 2).is_err());
    assert!(take(&s, "alice", "z", 2).is_err());
    s.release("r", "carol", "p/x").unwrap();
    take(&s, "alice", "p", 3).unwrap();
    s.release("r", "alice", "p").unwrap();
    // the wait for p/x is over (p covers it); the wait for z stands, so erin asking for alice's r is a deadlock
    take(&s, "dave", "p/x", 4).unwrap();
    assert!(!deadlock_flag(take(&s, "dave", "r", 5)));
    assert!(deadlock_flag(take(&s, "erin", "r", 5)));
}

/// alice waits for p and for x; bob holds x and p-to-be and waits for alice's q. Taking p completes a cycle for the
/// wait on p only: the wait on x was already a refused cycle, not one this take completed.
#[test]
fn a_take_marks_only_the_waits_it_overlaps() {
    let s = store();
    take(&s, "carol", "p", 0).unwrap();
    take(&s, "bob", "x", 0).unwrap();
    take(&s, "alice", "q", 1).unwrap();
    assert!(take(&s, "alice", "p", 2).is_err());
    assert!(take(&s, "alice", "x", 2).is_err());
    assert!(take(&s, "bob", "q", 3).is_err());
    s.release("r", "carol", "p").unwrap();
    take(&s, "bob", "p", 4).unwrap();
    assert_eq!(
        s.deadlock_notices("r", "alice", 5).unwrap(),
        [CycleNotice {
            path: "p".into(),
            holder: "bob".into()
        }]
    );
}

#[test]
fn a_take_by_a_member_that_waits_on_nobody_marks_nobody() {
    // alice waits for p; bob takes p. bob waits for nothing, so no cycle, whatever else alice holds or waits on.
    let s = store();
    take(&s, "carol", "p", 0).unwrap();
    take(&s, "alice", "q", 1).unwrap();
    take(&s, "dave", "q2", 1).unwrap();
    assert!(take(&s, "alice", "p", 2).is_err());
    assert!(take(&s, "alice", "q2", 2).is_err());
    s.release("r", "carol", "p").unwrap();
    take(&s, "bob", "p", 3).unwrap();
    assert!(s.deadlock_notices("r", "alice", 4).unwrap().is_empty());
}

#[test]
fn a_take_in_a_scope_that_does_not_collide_with_the_wait_marks_nothing() {
    // alice (s1) waits for p held by carol (s1). bob takes p in s2: no collision with alice's wait, so bob's claim
    // blocks nobody, even though bob waits for alice's q.
    let s = store();
    take_in(&s, ("carol", "s1"), "p", 0).unwrap();
    take_in(&s, ("alice", "s2"), "q", 1).unwrap();
    take_in(&s, ("erin", "s1"), "p/x", 1).unwrap_err();
    assert!(take_in(&s, ("alice", "s1"), "p", 2).is_err());
    assert!(take_in(&s, ("bob", "s2"), "q", 3).is_err());
    s.release("r", "carol", "p").unwrap();
    take_in(&s, ("bob", "s2"), "p", 4).unwrap();
    assert!(s.deadlock_notices("r", "alice", 5).unwrap().is_empty());
}

/// The cycle of `cycle_notices` tests: alice holds q for a long time; bob takes p at 3 for `ttl_p` seconds.
fn cycle_with_short_p(ttl_p: i64) -> SqliteStore {
    let s = store();
    take(&s, "carol", "p", 0).unwrap();
    s.claim(&ClaimRequest::new(("r", "alice"), "q", 5_000, ""), 0)
        .unwrap();
    assert!(take(&s, "alice", "p", 1).is_err());
    assert!(take(&s, "bob", "q", 2).is_err());
    s.release("r", "carol", "p").unwrap();
    s.claim(&ClaimRequest::new(("r", "bob"), "p", ttl_p, ""), 3)
        .unwrap();
    s
}

#[test]
fn a_notice_needs_the_holder_still_to_hold_its_claim_the_second_before_it_expires() {
    // bob's p lives to 13: at 12 it is held and the cycle is told; at 13 it has expired and there is no cycle
    let told = cycle_with_short_p(10).deadlock_notices("r", "alice", 12);
    assert_eq!(told.unwrap().len(), 1);
    let at_expiry = cycle_with_short_p(10).deadlock_notices("r", "alice", 13);
    assert!(at_expiry.unwrap().is_empty());
    let after = cycle_with_short_p(10).deadlock_notices("r", "alice", 14);
    assert!(after.unwrap().is_empty());
}

/// alice takes a at 1000, a statement changes its row, she takes it again at 5000: the row's (since, age_noted).
fn age_after_retake(update: &str) -> (SqliteStore, (i64, i64)) {
    let s = store();
    let take_a = |now| {
        let r = ClaimRequest::new(("r", "alice"), "a", 100_000, "");
        s.claim(&r, now).unwrap();
    };
    take_a(1_000);
    s.conn.execute(update, []).unwrap();
    take_a(5_000);
    let row = s
        .conn
        .query_row(
            "SELECT since, age_noted FROM claims WHERE path='a'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    (s, row)
}

#[test]
fn a_claim_taken_again_when_its_recorded_start_is_unknown_starts_a_new_age() {
    let (s, row) = age_after_retake("UPDATE claims SET since=0, age_noted=3 WHERE path='a'");
    assert_eq!(row, (5_000, 0));
    let aged = s.claim_ages("r", "alice", 5_000 + 7_200).unwrap();
    assert_eq!(aged.len(), 1);
    assert_eq!(aged[0].held_secs, 7_200);
}

#[test]
fn a_claim_taken_again_while_held_keeps_its_start_and_its_milestone() {
    let (_, row) = age_after_retake("UPDATE claims SET age_noted=3 WHERE path='a'");
    assert_eq!(row, (1_000, 3));
}

fn issue(s: &SqliteStore, member: &str, now: i64, ttl: i64) -> crate::domain::Result<()> {
    s.issue_credential(
        &NewCredential {
            member,
            token_hash: &[1; 32],
            clearance: "private",
            parent: "lead",
            ttl,
            max_life: 0,
        },
        now,
    )
}

#[test]
fn a_credential_is_live_until_the_second_before_it_expires_and_replaceable_from_the_second_it_does()
{
    // issued at 1000 for 600 s: it expires at 1600
    for (now, live) in [(1_599, true), (1_600, false), (1_601, false)] {
        let s = store();
        issue(&s, "w-1", 1_000, 600).unwrap();
        let again = issue(&s, "w-1", now, 600);
        assert_eq!(matches!(again, Err(Error::Invalid(_))), live, "at {now}");
        assert_eq!(again.is_ok(), !live, "at {now}");
        if !live {
            assert_eq!(s.credential("w-1").unwrap().unwrap().issued, now);
        }
    }
}

#[test]
fn a_revoked_credential_is_replaceable_even_before_it_expires() {
    let s = store();
    issue(&s, "w-1", 1_000, 600).unwrap();
    assert!(s.revoke_credential("w-1", 1_001).unwrap());
    issue(&s, "w-1", 1_002, 600).unwrap();
}
