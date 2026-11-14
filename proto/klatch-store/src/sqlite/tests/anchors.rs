//! Anchored claims (#125): a claim may name the work item it is for, and is released when that item is reported done.

use super::*;

fn claim(s: &SqliteStore, room: &str, owner: &str, path: &str, anchor: &str, now: i64) {
    s.claim(
        &ClaimRequest::new((room, owner), path, 600, "").anchored(anchor),
        now,
    )
    .unwrap();
}

#[test]
fn releasing_an_anchor_releases_its_live_claims_in_every_room_and_only_those() {
    let s = store();
    claim(&s, "r1", "w-1", "src/a.rs", "wo:7", 100);
    claim(&s, "r2", "w-2", "src/b.rs", "wo:7", 100);
    claim(&s, "r1", "w-3", "src/c.rs", "wo:8", 100);
    claim(&s, "r1", "w-4", "src/d.rs", "", 100);
    let mut released = s.release_anchored("wo:7", 200).unwrap();
    released.sort();
    assert_eq!(
        released,
        vec![
            ("r1".to_string(), "src/a.rs".to_string(), "w-1".to_string()),
            ("r2".to_string(), "src/b.rs".to_string(), "w-2".to_string()),
        ]
    );
    let held: Vec<String> = s
        .claims("r1", 200)
        .unwrap()
        .into_iter()
        .map(|c| c.path)
        .collect();
    assert_eq!(held, vec!["src/c.rs".to_string(), "src/d.rs".to_string()]);
    assert!(s.claims("r2", 200).unwrap().is_empty());
    assert!(
        s.release_anchored("wo:7", 200).unwrap().is_empty(),
        "a second release found something"
    );
}

#[test]
fn an_expired_claim_is_cleared_but_not_reported_as_released() {
    let s = store();
    claim(&s, "r", "w-1", "src/a.rs", "wo:7", 100); // expires at 700
    assert!(s.release_anchored("wo:7", 800).unwrap().is_empty());
    assert!(s.claims("r", 800).unwrap().is_empty());
}

#[test]
fn a_released_path_can_be_claimed_by_someone_else_at_once() {
    let s = store();
    claim(&s, "r", "w-1", "src/a.rs", "wo:7", 100);
    assert!(
        s.claim(&ClaimRequest::new(("r", "w-2"), "src/a.rs", 600, ""), 150)
            .is_err()
    );
    s.release_anchored("wo:7", 160).unwrap();
    s.claim(&ClaimRequest::new(("r", "w-2"), "src/a.rs", 600, ""), 170)
        .unwrap();
}

#[test]
fn the_anchors_in_use_are_the_distinct_anchors_of_live_claims() {
    let s = store();
    claim(&s, "r", "w-1", "a", "wo:2", 100);
    claim(&s, "r", "w-2", "b", "wo:1", 100);
    claim(&s, "r", "w-3", "c", "wo:1", 100);
    claim(&s, "r", "w-4", "d", "", 100);
    claim(&s, "r", "w-5", "e", "wo:gone", 10); // expires at 610
    assert_eq!(
        s.anchors_in_use(650).unwrap(),
        vec!["wo:1".to_string(), "wo:2".to_string()]
    );
}

#[test]
fn a_database_from_before_anchors_gains_the_column_and_its_claims_have_none() {
    let path = temp_db("anchor-migrate");
    drop(SqliteStore::open(&path, &crate::insecure_test_key()).unwrap());
    {
        let c = keyed(&path);
        c.execute_batch(
            "INSERT INTO claims(room,path,owner,member,reason,expires,scope) VALUES('r','p','w','w','',9999999999,'');
             ALTER TABLE claims DROP COLUMN anchor; PRAGMA user_version = 21;",
        )
        .unwrap();
    }
    let s = SqliteStore::open(&path, &crate::insecure_test_key()).unwrap();
    assert_eq!(s.claims("r", 1).unwrap().len(), 1);
    assert!(s.anchors_in_use(1).unwrap().is_empty());
}
