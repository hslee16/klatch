//! Issued credentials (#121, #120, #161): the registry that makes "verified" mean "holds a credential the issuer
//! made and has not revoked or let expire". The store keeps only a hash of the token.

use super::*;
use crate::store::NewCredential;

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
fn a_database_from_before_credentials_gains_the_table_on_open() {
    let path = temp_db("cred-migrate");
    drop(SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap());
    {
        let c = keyed(&path);
        c.execute_batch(
            "DROP TABLE credentials; ALTER TABLE messages DROP COLUMN kind; ALTER TABLE messages DROP COLUMN payload; ALTER TABLE claims DROP COLUMN anchor; PRAGMA user_version = 19;",
        )
        .unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    issue(&s, "w-1", 1, 10).unwrap();
    assert!(s.credential("w-1").unwrap().is_some());
}
