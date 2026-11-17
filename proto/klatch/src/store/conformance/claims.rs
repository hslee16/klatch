use super::*;
use crate::domain::Error;
/// Alice holds X (in `rooms.0`) and bob holds Y (in `rooms.1`); alice then asks for Y and bob for X, at the times in
/// `asks_at`. Returns whether each ask was refused as a deadlock.
fn stand_off<S: Store>(
    s: &S,
    rooms: (&str, &str),
    ttl: i64,
    asks_at: (i64, i64),
) -> (Option<bool>, Option<bool>) {
    take(s, (rooms.0, "alice"), "X", Lease::new(0, ttl));
    take(s, (rooms.1, "bob"), "Y", Lease::new(0, ttl));
    (
        refused_as_deadlock(s, (rooms.1, "alice"), "Y", Lease::new(asks_at.0, ttl)),
        refused_as_deadlock(s, (rooms.0, "bob"), "X", Lease::new(asks_at.1, ttl)),
    )
}

/// Bob holds `a` for `ttl` and alice has already seen it, so her next view starts from there.
fn bob_holds_a_and_alice_has_seen_it<S: Store>(s: &S, ttl: i64) {
    take(s, ("r", "bob"), "a", Lease::new(0, ttl));
    changes(s, "r", "alice", 1);
}

fn alice_sees<S: Store>(s: &S, now: i64, added: &[(&str, &str)], released: &[(&str, &str)]) {
    let own = |v: &[(&str, &str)]| v.iter().map(|(o, p)| pair(o, p)).collect::<Pairs>();
    assert_eq!(changes(s, "r", "alice", now), (own(added), own(released)));
}

fn alice_sees_nothing_new<S: Store>(s: &S, now: i64) {
    assert_eq!(changes(s, "r", "alice", now), (vec![], vec![]), "at {now}");
}

pub fn the_attempt_that_closes_a_two_party_cycle_is_told_it_is_a_deadlock<S: Store>(
    make: fn() -> S,
) {
    let s = make();
    let flags = stand_off(&s, ("r", "r"), 1000, (10, 20));
    assert_eq!(
        flags,
        (Some(false), Some(true)),
        "no cycle at the first wait; bob closed it"
    );
}
pub fn a_plain_conflict_is_not_a_deadlock<S: Store>(make: fn() -> S) {
    let s = make();
    take(&s, ("r", "alice"), "X", Lease::new(0, 1000));
    assert_eq!(
        refused_as_deadlock(&s, ("r", "bob"), "X", Lease::new(5, 1000)),
        Some(false)
    );
    take(&s, ("r", "bob"), "Y", Lease::new(6, 1000));
    assert_eq!(
        refused_as_deadlock(&s, ("r", "bob"), "X", Lease::new(7, 1000)),
        Some(false),
        "alice is not waiting on bob"
    );
}
pub fn a_wait_older_than_the_window_does_not_make_a_cycle<S: Store>(make: fn() -> S) {
    let s = make();
    let flags = stand_off(&s, ("r", "r"), 5000, (10, 10 + 121));
    assert_eq!(
        flags,
        (Some(false), Some(false)),
        "alice stopped waiting long ago"
    );
}
pub fn a_cycle_of_three_is_found_at_the_attempt_that_closes_it<S: Store>(make: fn() -> S) {
    let s = make();
    take(&s, ("r", "a"), "X", Lease::new(0, 1000));
    take(&s, ("r", "b"), "Y", Lease::new(0, 1000));
    take(&s, ("r", "c"), "Z", Lease::new(0, 1000));
    assert_eq!(
        refused_as_deadlock(&s, ("r", "a"), "Y", Lease::new(5, 1000)),
        Some(false)
    );
    assert_eq!(
        refused_as_deadlock(&s, ("r", "b"), "Z", Lease::new(6, 1000)),
        Some(false)
    );
    assert_eq!(
        refused_as_deadlock(&s, ("r", "c"), "X", Lease::new(7, 1000)),
        Some(true)
    );
}
pub fn a_satisfied_wait_no_longer_counts<S: Store>(make: fn() -> S) {
    let s = make();
    take(&s, ("r", "alice"), "X", Lease::new(0, 1000));
    take(&s, ("r", "bob"), "Y", Lease::new(0, 1000));
    assert_eq!(
        refused_as_deadlock(&s, ("r", "alice"), "Y", Lease::new(5, 1000)),
        Some(false)
    );
    s.release("r", "bob", "Y").unwrap();
    take(&s, ("r", "alice"), "Y", Lease::new(6, 1000));
    assert_eq!(
        refused_as_deadlock(&s, ("r", "bob"), "X", Lease::new(7, 1000)),
        Some(false),
        "alice got Y, so she is not waiting"
    );
}
pub fn waits_in_one_room_do_not_make_a_cycle_in_another<S: Store>(make: fn() -> S) {
    let s = make();
    let flags = stand_off(&s, ("r1", "r2"), 1000, (5, 6));
    assert_eq!(flags, (Some(false), Some(false)));
}
pub fn a_shared_claim_collides_with_every_scope_in_both_directions<S: Store>(make: fn() -> S) {
    let s = make();
    s.claim(
        &ClaimRequest::new(("r", "alice"), "docs", 100, "").scoped("*"),
        0,
    )
    .unwrap();
    assert!(
        s.claim(
            &ClaimRequest::new(("r", "bob"), "docs/api.md", 100, "").scoped("git:b"),
            1
        )
        .is_err(),
        "scoped against shared"
    );
    s.claim(
        &ClaimRequest::new(("r", "carol"), "src", 100, "").scoped("git:c"),
        2,
    )
    .unwrap();
    assert!(
        s.claim(
            &ClaimRequest::new(("r", "dave"), "src/x.rs", 100, "").scoped("*"),
            3
        )
        .is_err(),
        "shared against scoped"
    );
}
pub fn an_empty_scope_behaves_as_claims_always_did_and_the_listing_shows_each_scope<S: Store>(
    make: fn() -> S,
) {
    let s = make();
    take(&s, ("r", "alice"), "p", Lease::new(0, 100));
    assert!(
        s.claim(&ClaimRequest::new(("r", "bob"), "p", 100, ""), 1)
            .is_err(),
        "the default scope still collides"
    );
    s.claim(
        &ClaimRequest::new(("r", "carol"), "q", 100, "").scoped("git:c"),
        2,
    )
    .unwrap();
    let claims = s.claims("r", 3).unwrap();
    let scope_of = |p: &str| claims.iter().find(|c| c.path == p).map(|c| c.scope.clone());
    assert_eq!(scope_of("p").as_deref(), Some(""));
    assert_eq!(scope_of("q").as_deref(), Some("git:c"));
}
pub fn a_release_note_travels_to_those_who_saw_the_claim_and_to_the_next_claimant<S: Store>(
    make: fn() -> S,
) {
    let s = make();
    take(&s, ("r", "bob"), "src/a.rs", Lease::new(0, 100));
    assert_eq!(changes(&s, "r", "alice", 1).0.len(), 1);
    s.release_with(
        &Release::new(("r", "bob"), "src/a.rs", "rewrote parse(); re-read it"),
        10,
    )
    .unwrap();
    let c = s.claim_changes("r", "alice", 11).unwrap();
    assert_eq!(c.released.len(), 1);
    assert_eq!(
        c.released[0].note.as_deref(),
        Some("rewrote parse(); re-read it")
    );
    let last = s.last_release("r", "src/a.rs").unwrap().unwrap();
    assert_eq!(
        (last.owner.as_str(), last.note.as_str(), last.ts),
        ("bob", "rewrote parse(); re-read it", 10)
    );
    // a later release on the same path replaces it; one with no note leaves no note behind
    take(&s, ("r", "carol"), "src/a.rs", Lease::new(20, 100));
    s.release_with(&Release::new(("r", "carol"), "src/a.rs", ""), 30)
        .unwrap();
    assert!(s.last_release("r", "src/a.rs").unwrap().is_none());
    assert!(s.last_release("r", "other.rs").unwrap().is_none());
}
pub fn removing_a_member_takes_it_out_of_the_room_and_releases_its_claims_there<S: Store>(
    make: fn() -> S,
) {
    let s = make();
    s.join("r", "mallory", 1).unwrap();
    s.join("side", "mallory", 1).unwrap();
    take(&s, ("r", "mallory"), "a.rs", Lease::new(2, 600));
    take(&s, ("side", "mallory"), "b.rs", Lease::new(2, 600));
    assert_eq!(s.remove_member("r", "mallory", 3).unwrap(), 1);
    assert!(s.members("r").unwrap().iter().all(|m| m.name != "mallory"));
    assert!(s.claims("r", 4).unwrap().is_empty());
    assert_eq!(
        s.claims("side", 4).unwrap().len(),
        1,
        "the other room is untouched"
    );
    assert_eq!(s.rooms_of("mallory").unwrap(), vec!["side".to_string()]);
}
pub fn claims_do_not_conflict_across_rooms<S: Store>(make: fn() -> S) {
    let s = make();
    take(&s, ("r1", "bob"), "src", Lease::new(0, 100));
    take(&s, ("r2", "alice"), "src", Lease::new(0, 100));
}
pub fn an_owner_can_reclaim_and_replace_their_own_path<S: Store>(make: fn() -> S) {
    let s = make();
    s.claim(&ClaimRequest::new(("r", "bob"), "src", 100, "a"), 0)
        .unwrap();
    s.claim(&ClaimRequest::new(("r", "bob"), "src", 200, "b"), 0)
        .unwrap();
    let c = s.claims("r", 1).unwrap();
    assert_eq!(
        c,
        vec![Claim {
            path: "src".into(),
            owner: "bob".into(),
            reason: "b".into(),
            expires: 200,
            scope: String::new()
        }]
    );
}
pub fn expired_claims_free_the_path_and_are_not_listed<S: Store>(make: fn() -> S) {
    let s = make();
    take(&s, ("r", "bob"), "src", Lease::new(0, 10));
    assert!(s.claims("r", 10).unwrap().is_empty());
    take(&s, ("r", "alice"), "src", Lease::new(10, 10));
    assert_eq!(s.claims("r", 11).unwrap()[0].owner, "alice");
}
pub fn claims_are_ordered_by_path<S: Store>(make: fn() -> S) {
    let s = make();
    take(&s, ("r", "bob"), "z", Lease::new(0, 100));
    take(&s, ("r", "bob"), "a", Lease::new(0, 100));
    let paths: Vec<_> = s
        .claims("r", 1)
        .unwrap()
        .into_iter()
        .map(|c| c.path)
        .collect();
    assert_eq!(paths, vec!["a", "z"]);
}
pub fn claim_changes_first_call_shows_others_claims_then_nothing<S: Store>(make: fn() -> S) {
    let s = make();
    take(&s, ("r", "bob"), "a", Lease::new(0, 100));
    take(&s, ("r", "alice"), "mine", Lease::new(0, 100));
    alice_sees(&s, 1, &[("bob", "a")], &[]);
    alice_sees(&s, 2, &[], &[]);
}
pub fn claim_changes_ignores_the_passing_of_time<S: Store>(make: fn() -> S) {
    let s = make();
    bob_holds_a_and_alice_has_seen_it(&s, 1000);
    for now in [10, 100, 500] {
        alice_sees_nothing_new(&s, now);
    }
}
pub fn claim_changes_reports_only_the_new_claim<S: Store>(make: fn() -> S) {
    let s = make();
    bob_holds_a_and_alice_has_seen_it(&s, 100);
    take(&s, ("r", "bob"), "b", Lease::new(2, 100));
    alice_sees(&s, 3, &[("bob", "b")], &[]);
}
pub fn claim_changes_reports_a_renewed_claim_again<S: Store>(make: fn() -> S) {
    let s = make();
    bob_holds_a_and_alice_has_seen_it(&s, 100);
    take(&s, ("r", "bob"), "a", Lease::new(50, 500));
    alice_sees(&s, 51, &[("bob", "a")], &[]);
    alice_sees_nothing_new(&s, 52);
}
pub fn claim_changes_reports_a_release_once<S: Store>(make: fn() -> S) {
    let s = make();
    bob_holds_a_and_alice_has_seen_it(&s, 100);
    s.release("r", "bob", "a").unwrap();
    alice_sees(&s, 2, &[], &[("bob", "a")]);
    alice_sees_nothing_new(&s, 3);
}
pub fn claim_changes_are_remembered_per_member_and_per_room<S: Store>(make: fn() -> S) {
    let s = make();
    take(&s, ("r", "bob"), "a", Lease::new(0, 100));
    take(&s, ("other", "bob"), "z", Lease::new(0, 100));
    assert_eq!(changes(&s, "r", "alice", 1).0.len(), 1);
    assert_eq!(
        changes(&s, "r", "carol", 1).0.len(),
        1,
        "carol has her own view"
    );
    assert_eq!(
        changes(&s, "other", "alice", 1).0.len(),
        1,
        "another room has its own view"
    );
    assert_eq!(changes(&s, "r", "alice", 2).0.len(), 0);
}
pub fn claim_changes_does_not_hold_the_writer_lock_after_it_returns<S: Store>(make: fn() -> S) {
    let s = make();
    take(&s, ("r", "bob"), "a", Lease::new(0, 100));
    changes(&s, "r", "alice", 1);
    take(&s, ("r", "bob"), "b", Lease::new(2, 100));
}
