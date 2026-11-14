//! Issued credentials (#121, #120, #161): the registry that makes "verified" mean "holds a credential the issuer
//! made and has not revoked or let expire". The store keeps only a hash of the token.

use super::*;
use crate::NewCredential;

fn hash(b: u8) -> [u8; 32] {
    [b; 32]
}

fn issue(s: &SqliteStore, member: &str, now: i64, ttl: i64) -> Result<()> {
    s.issue_credential(
        &NewCredential {
            member,
            token_hash: &hash(1),
            clearance: "private",
            parent: "lead",
            ttl,
        },
        now,
    )
}

#[test]
fn an_issued_credential_is_stored_with_its_hash_clearance_parent_and_expiry() {
    let s = store();
    issue(&s, "w-1", 1_000, 600).unwrap();
    let c = s.credential("w-1").unwrap().unwrap();
    assert_eq!(c.token_hash, hash(1).to_vec());
    assert_eq!(
        (c.clearance.as_str(), c.parent.as_str()),
        ("private", "lead")
    );
    assert_eq!((c.issued, c.expires, c.revoked), (1_000, 1_600, false));
    assert!(s.credential("nobody").unwrap().is_none());
}

#[test]
fn a_member_with_a_live_credential_cannot_be_issued_another_until_it_is_revoked_or_expires() {
    let s = store();
    issue(&s, "w-1", 1_000, 600).unwrap();
    assert!(matches!(
        issue(&s, "w-1", 1_100, 600),
        Err(Error::Invalid(_))
    ));
    // expired: a new one replaces it
    issue(&s, "w-1", 1_601, 600).unwrap();
    assert_eq!(s.credential("w-1").unwrap().unwrap().issued, 1_601);
    // revoked: also replaceable
    assert!(s.revoke_credential("w-1", 1_700).unwrap());
    issue(&s, "w-1", 1_701, 600).unwrap();
    assert!(!s.credential("w-1").unwrap().unwrap().revoked);
}

#[test]
fn revoking_works_once_and_marks_the_record_without_deleting_it() {
    let s = store();
    issue(&s, "w-1", 1_000, 600).unwrap();
    assert!(s.revoke_credential("w-1", 1_100).unwrap());
    assert!(!s.revoke_credential("w-1", 1_200).unwrap());
    assert!(!s.revoke_credential("never-issued", 1_200).unwrap());
    assert!(s.credential("w-1").unwrap().unwrap().revoked);
}

#[test]
fn the_list_has_every_credential_by_member() {
    let s = store();
    issue(&s, "w-2", 1_000, 600).unwrap();
    issue(&s, "w-1", 1_000, 600).unwrap();
    let names: Vec<String> = s
        .credentials()
        .unwrap()
        .into_iter()
        .map(|c| c.member)
        .collect();
    assert_eq!(names, vec!["w-1".to_string(), "w-2".to_string()]);
}

#[test]
fn a_database_from_before_credentials_gains_the_table_on_open() {
    let path = temp_db("cred-migrate");
    drop(SqliteStore::open(&path, &crate::insecure_test_key()).unwrap());
    {
        let c = keyed(&path);
        c.execute_batch("DROP TABLE credentials; PRAGMA user_version = 19;")
            .unwrap();
    }
    let s = SqliteStore::open(&path, &crate::insecure_test_key()).unwrap();
    issue(&s, "w-1", 1, 10).unwrap();
    assert!(s.credential("w-1").unwrap().is_some());
}
