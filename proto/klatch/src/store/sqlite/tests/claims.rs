use super::*;
use crate::domain::Error;

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
fn a_members_scope_is_remembered_and_the_last_one_wins() {
    let path = temp_db("member-scope");
    {
        let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
        assert_eq!(s.member_scope("alice").unwrap(), None);
        s.set_member_scope("alice", "git:a", 1).unwrap();
        s.set_member_scope("alice", "bot:grok", 2).unwrap();
        s.set_member_scope("bob", "git:b", 3).unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    assert_eq!(
        s.member_scope("alice").unwrap().as_deref(),
        Some("bot:grok")
    );
    assert_eq!(s.member_scope("bob").unwrap().as_deref(), Some("git:b"));
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
