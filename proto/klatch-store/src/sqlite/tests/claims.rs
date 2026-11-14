use super::*;
use klatch_core::Error;

#[test]
fn the_attempt_that_closes_a_two_party_cycle_is_told_it_is_a_deadlock() {
    let s = store();
    s.claim(&ClaimRequest::new(("r", "alice"), "X", 1000, ""), 0)
        .unwrap();
    s.claim(&ClaimRequest::new(("r", "bob"), "Y", 1000, ""), 0)
        .unwrap();
    assert_eq!(
        deadlocked(s.claim(&ClaimRequest::new(("r", "alice"), "Y", 1000, ""), 10)),
        Some(false),
        "no cycle yet"
    );
    assert_eq!(
        deadlocked(s.claim(&ClaimRequest::new(("r", "bob"), "X", 1000, ""), 20)),
        Some(true),
        "bob closed the cycle"
    );
}

#[test]
fn a_plain_conflict_is_not_a_deadlock() {
    let s = store();
    s.claim(&ClaimRequest::new(("r", "alice"), "X", 1000, ""), 0)
        .unwrap();
    assert_eq!(
        deadlocked(s.claim(&ClaimRequest::new(("r", "bob"), "X", 1000, ""), 5)),
        Some(false)
    );
    s.claim(&ClaimRequest::new(("r", "bob"), "Y", 1000, ""), 6)
        .unwrap();
    assert_eq!(
        deadlocked(s.claim(&ClaimRequest::new(("r", "bob"), "X", 1000, ""), 7)),
        Some(false),
        "alice is not waiting on bob"
    );
}

#[test]
fn a_wait_older_than_the_window_does_not_make_a_cycle() {
    let s = store();
    s.claim(&ClaimRequest::new(("r", "alice"), "X", 5000, ""), 0)
        .unwrap();
    s.claim(&ClaimRequest::new(("r", "bob"), "Y", 5000, ""), 0)
        .unwrap();
    assert_eq!(
        deadlocked(s.claim(&ClaimRequest::new(("r", "alice"), "Y", 5000, ""), 10)),
        Some(false)
    );
    assert_eq!(
        deadlocked(s.claim(&ClaimRequest::new(("r", "bob"), "X", 5000, ""), 10 + 121)),
        Some(false),
        "alice stopped waiting long ago"
    );
}

#[test]
fn a_cycle_of_three_is_found_at_the_attempt_that_closes_it() {
    let s = store();
    s.claim(&ClaimRequest::new(("r", "a"), "X", 1000, ""), 0)
        .unwrap();
    s.claim(&ClaimRequest::new(("r", "b"), "Y", 1000, ""), 0)
        .unwrap();
    s.claim(&ClaimRequest::new(("r", "c"), "Z", 1000, ""), 0)
        .unwrap();
    assert_eq!(
        deadlocked(s.claim(&ClaimRequest::new(("r", "a"), "Y", 1000, ""), 5)),
        Some(false)
    );
    assert_eq!(
        deadlocked(s.claim(&ClaimRequest::new(("r", "b"), "Z", 1000, ""), 6)),
        Some(false)
    );
    assert_eq!(
        deadlocked(s.claim(&ClaimRequest::new(("r", "c"), "X", 1000, ""), 7)),
        Some(true)
    );
}

#[test]
fn a_satisfied_wait_no_longer_counts() {
    let s = store();
    s.claim(&ClaimRequest::new(("r", "alice"), "X", 1000, ""), 0)
        .unwrap();
    s.claim(&ClaimRequest::new(("r", "bob"), "Y", 1000, ""), 0)
        .unwrap();
    assert_eq!(
        deadlocked(s.claim(&ClaimRequest::new(("r", "alice"), "Y", 1000, ""), 5)),
        Some(false)
    );
    s.release("r", "bob", "Y").unwrap();
    s.claim(&ClaimRequest::new(("r", "alice"), "Y", 1000, ""), 6)
        .unwrap();
    assert_eq!(
        deadlocked(s.claim(&ClaimRequest::new(("r", "bob"), "X", 1000, ""), 7)),
        Some(false),
        "alice got Y, so she is not waiting"
    );
}

#[test]
fn waits_in_one_room_do_not_make_a_cycle_in_another() {
    let s = store();
    s.claim(&ClaimRequest::new(("r1", "alice"), "X", 1000, ""), 0)
        .unwrap();
    s.claim(&ClaimRequest::new(("r2", "bob"), "Y", 1000, ""), 0)
        .unwrap();
    assert_eq!(
        deadlocked(s.claim(&ClaimRequest::new(("r2", "alice"), "Y", 1000, ""), 5)),
        Some(false)
    );
    assert_eq!(
        deadlocked(s.claim(&ClaimRequest::new(("r1", "bob"), "X", 1000, ""), 6)),
        Some(false)
    );
}

#[test]
fn claims_in_different_scopes_do_not_collide_but_the_same_scope_does() {
    let s = store();
    s.claim(
        &ClaimRequest::new(("r", "alice"), "src/lib.rs", 100, "").scoped("git:a"),
        0,
    )
    .unwrap();
    s.claim(
        &ClaimRequest::new(("r", "bob"), "src/lib.rs", 100, "").scoped("git:b"),
        0,
    )
    .unwrap();
    let err = s
        .claim(
            &ClaimRequest::new(("r", "carol"), "src/lib.rs", 100, "").scoped("git:a"),
            1,
        )
        .unwrap_err();
    assert!(
        matches!(err, Error::Conflict { ref owner, .. } if owner == "alice"),
        "{err}"
    );
}

#[test]
fn a_shared_claim_collides_with_every_scope_in_both_directions() {
    let s = store();
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

#[test]
fn an_empty_scope_behaves_as_claims_always_did_and_the_listing_shows_each_scope() {
    let s = store();
    s.claim(&ClaimRequest::new(("r", "alice"), "p", 100, ""), 0)
        .unwrap();
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

#[test]
fn a_members_scope_is_remembered_and_the_last_one_wins() {
    let path = temp_db("member-scope");
    {
        let s = SqliteStore::open(&path).unwrap();
        assert_eq!(s.member_scope("alice").unwrap(), None);
        s.set_member_scope("alice", "git:a", 1).unwrap();
        s.set_member_scope("alice", "bot:grok", 2).unwrap();
        s.set_member_scope("bob", "git:b", 3).unwrap();
    }
    let s = SqliteStore::open(&path).unwrap();
    assert_eq!(
        s.member_scope("alice").unwrap().as_deref(),
        Some("bot:grok")
    );
    assert_eq!(s.member_scope("bob").unwrap().as_deref(), Some("git:b"));
}

#[test]
fn a_release_note_travels_to_those_who_saw_the_claim_and_to_the_next_claimant() {
    let s = store();
    s.claim(&ClaimRequest::new(("r", "bob"), "src/a.rs", 100, ""), 0)
        .unwrap();
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
    s.claim(&ClaimRequest::new(("r", "carol"), "src/a.rs", 100, ""), 20)
        .unwrap();
    s.release_with(&Release::new(("r", "carol"), "src/a.rs", ""), 30)
        .unwrap();
    assert!(s.last_release("r", "src/a.rs").unwrap().is_none());
    assert!(s.last_release("r", "other.rs").unwrap().is_none());
}

#[test]
fn removing_a_member_takes_it_out_of_the_room_and_releases_its_claims_there() {
    let s = store();
    s.join("r", "mallory", 1).unwrap();
    s.join("side", "mallory", 1).unwrap();
    s.claim(&ClaimRequest::new(("r", "mallory"), "a.rs", 600, ""), 2)
        .unwrap();
    s.claim(&ClaimRequest::new(("side", "mallory"), "b.rs", 600, ""), 2)
        .unwrap();
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

#[test]
fn claim_conflicts_with_an_overlapping_claim_by_another() {
    let s = store();
    s.claim(&ClaimRequest::new(("r", "bob"), "src", 100, "refactor"), 0)
        .unwrap();
    let err = s
        .claim(&ClaimRequest::new(("r", "alice"), "src/a.rs", 100, ""), 10)
        .unwrap_err();
    assert_eq!(
        err,
        Error::Conflict {
            path: "src".into(),
            owner: "bob".into(),
            secs_left: 90,
            deadlock: false
        }
    );
    s.claim(&ClaimRequest::new(("r", "alice"), "docs", 100, ""), 10)
        .unwrap(); // disjoint is fine
}

#[test]
fn claims_do_not_conflict_across_rooms() {
    let s = store();
    s.claim(&ClaimRequest::new(("r1", "bob"), "src", 100, ""), 0)
        .unwrap();
    s.claim(&ClaimRequest::new(("r2", "alice"), "src", 100, ""), 0)
        .unwrap();
}

#[test]
fn an_owner_can_reclaim_and_replace_their_own_path() {
    let s = store();
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

#[test]
fn expired_claims_free_the_path_and_are_not_listed() {
    let s = store();
    s.claim(&ClaimRequest::new(("r", "bob"), "src", 10, ""), 0)
        .unwrap();
    assert!(s.claims("r", 10).unwrap().is_empty());
    s.claim(&ClaimRequest::new(("r", "alice"), "src", 10, ""), 10)
        .unwrap();
    assert_eq!(s.claims("r", 11).unwrap()[0].owner, "alice");
}

#[test]
fn claims_are_ordered_by_path() {
    let s = store();
    s.claim(&ClaimRequest::new(("r", "bob"), "z", 100, ""), 0)
        .unwrap();
    s.claim(&ClaimRequest::new(("r", "bob"), "a", 100, ""), 0)
        .unwrap();
    let paths: Vec<_> = s
        .claims("r", 1)
        .unwrap()
        .into_iter()
        .map(|c| c.path)
        .collect();
    assert_eq!(paths, vec!["a", "z"]);
}

#[test]
fn claim_changes_first_call_shows_others_claims_then_nothing() {
    let s = store();
    s.claim(&ClaimRequest::new(("r", "bob"), "a", 100, ""), 0)
        .unwrap();
    s.claim(&ClaimRequest::new(("r", "alice"), "mine", 100, ""), 0)
        .unwrap();
    assert_eq!(
        changes(&s, "r", "alice", 1),
        (vec![pair("bob", "a")], vec![])
    );
    assert_eq!(changes(&s, "r", "alice", 2), (vec![], vec![]));
}

#[test]
fn claim_changes_ignores_the_passing_of_time() {
    let s = store();
    s.claim(&ClaimRequest::new(("r", "bob"), "a", 1000, ""), 0)
        .unwrap();
    changes(&s, "r", "alice", 1);
    for now in [10, 100, 500] {
        assert_eq!(changes(&s, "r", "alice", now), (vec![], vec![]), "at {now}");
    }
}

#[test]
fn claim_changes_reports_only_the_new_claim() {
    let s = store();
    s.claim(&ClaimRequest::new(("r", "bob"), "a", 100, ""), 0)
        .unwrap();
    changes(&s, "r", "alice", 1);
    s.claim(&ClaimRequest::new(("r", "bob"), "b", 100, ""), 2)
        .unwrap();
    assert_eq!(
        changes(&s, "r", "alice", 3),
        (vec![pair("bob", "b")], vec![])
    );
}

#[test]
fn claim_changes_reports_a_renewed_claim_again() {
    let s = store();
    s.claim(&ClaimRequest::new(("r", "bob"), "a", 100, ""), 0)
        .unwrap();
    changes(&s, "r", "alice", 1);
    s.claim(&ClaimRequest::new(("r", "bob"), "a", 500, ""), 50)
        .unwrap();
    assert_eq!(
        changes(&s, "r", "alice", 51),
        (vec![pair("bob", "a")], vec![])
    );
    assert_eq!(changes(&s, "r", "alice", 52), (vec![], vec![]));
}

#[test]
fn claim_changes_reports_a_release_once() {
    let s = store();
    s.claim(&ClaimRequest::new(("r", "bob"), "a", 100, ""), 0)
        .unwrap();
    changes(&s, "r", "alice", 1);
    s.release("r", "bob", "a").unwrap();
    assert_eq!(
        changes(&s, "r", "alice", 2),
        (vec![], vec![pair("bob", "a")])
    );
    assert_eq!(changes(&s, "r", "alice", 3), (vec![], vec![]));
}

#[test]
fn claim_changes_are_remembered_per_member_and_per_room() {
    let s = store();
    s.claim(&ClaimRequest::new(("r", "bob"), "a", 100, ""), 0)
        .unwrap();
    s.claim(&ClaimRequest::new(("other", "bob"), "z", 100, ""), 0)
        .unwrap();
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

#[test]
fn claim_changes_does_not_hold_the_writer_lock_after_it_returns() {
    let s = store();
    s.claim(&ClaimRequest::new(("r", "bob"), "a", 100, ""), 0)
        .unwrap();
    changes(&s, "r", "alice", 1);
    s.claim(&ClaimRequest::new(("r", "bob"), "b", 100, ""), 2)
        .unwrap();
}

#[test]
fn release_removes_only_your_exact_claim() {
    let s = store();
    s.claim(&ClaimRequest::new(("r", "bob"), "src", 100, ""), 0)
        .unwrap();
    assert!(matches!(
        s.release("r", "alice", "src"),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        s.release("r", "bob", "other"),
        Err(Error::NotFound(_))
    ));
    s.release("r", "bob", "src").unwrap();
    assert!(s.claims("r", 1).unwrap().is_empty());
}
