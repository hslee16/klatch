use super::*;
use crate::domain::Error;

fn expiry_of(s: &SqliteStore, path: &str) -> i64 {
    s.conn
        .query_row("SELECT expires FROM claims WHERE path=?1", [path], |r| {
            r.get(0)
        })
        .unwrap()
}

#[test]
fn the_ttl_a_claim_was_made_with_is_stored_with_it() {
    let s = store();
    s.claim(&ClaimRequest::new(("r", "alice"), "a", 77, ""), 5)
        .unwrap();
    let ttl: i64 = s
        .conn
        .query_row("SELECT ttl FROM claims WHERE path='a'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ttl, 77);
}

#[test]
fn renewing_on_activity_never_shortens_a_stretched_lease() {
    let s = store();
    s.claim(&ClaimRequest::new(("r", "alice"), "a", 60, ""), 0)
        .unwrap();
    // A park or a liveness lease stretched it far past its own ttl.
    s.retime_member_claims("alice", 5_000, true).unwrap();
    s.touch("r", "alice", 10).unwrap();
    assert_eq!(expiry_of(&s, "a"), 5_000);
    s.touch("r", "alice", 4_990).unwrap();
    assert_eq!(
        expiry_of(&s, "a"),
        5_050,
        "past the stretch it follows its own ttl"
    );
}

#[test]
fn a_claim_from_before_the_ttl_was_stored_is_not_renewed() {
    let s = store();
    s.claim(&ClaimRequest::new(("r", "alice"), "a", 60, ""), 0)
        .unwrap();
    s.conn.execute_batch("UPDATE claims SET ttl=0").unwrap();
    s.touch("r", "alice", 30).unwrap();
    assert_eq!(expiry_of(&s, "a"), 60);
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
            held_secs: 0, // taken at time 0, which reads as unknown
            deadlock: false,
            cycle: vec![],
            via: String::new()
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
