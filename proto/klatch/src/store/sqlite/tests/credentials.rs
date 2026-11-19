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
            max_life: 0,
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

fn issue_renewable(s: &SqliteStore, now: i64, ttl: i64, max_life: i64) {
    s.issue_credential(
        &NewCredential {
            member: "w-1",
            token_hash: &hash(1),
            clearance: "private",
            parent: "lead",
            ttl,
            max_life,
        },
        now,
    )
    .unwrap();
}

#[test]
fn a_renewable_credential_starts_at_the_idle_timeout_or_the_cap_whichever_is_nearer() {
    let s = store();
    issue_renewable(&s, 1_000, 600, 5_000);
    let c = s.credential("w-1").unwrap().unwrap();
    assert_eq!((c.expires, c.idle, c.cap), (1_600, 600, 6_000));
    assert!(s.revoke_credential("w-1", 1_001).unwrap());
    issue_renewable(&s, 1_000, 600, 300);
    assert_eq!(s.credential("w-1").unwrap().unwrap().expires, 1_300);
}

#[test]
fn renewal_moves_the_expiry_to_now_plus_idle_and_stops_at_the_cap() {
    let s = store();
    issue_renewable(&s, 1_000, 600, 1_000);
    assert_eq!(s.renew_credential("w-1", 1_300, 60).unwrap(), Some(1_900));
    assert_eq!(s.renew_credential("w-1", 1_700, 60).unwrap(), Some(2_000));
    // at the cap there is nothing more to give
    assert_eq!(s.renew_credential("w-1", 1_800, 60).unwrap(), None);
    assert_eq!(s.credential("w-1").unwrap().unwrap().expires, 2_000);
}

#[test]
fn renewal_writes_only_for_a_gain_of_a_step_or_one_that_reaches_the_cap() {
    let s = store();
    issue_renewable(&s, 1_000, 600, 1_000);
    // 1010 + 600 = 1610 gains 10 over 1600: under the step, left alone
    assert_eq!(s.renew_credential("w-1", 1_010, 60).unwrap(), None);
    assert_eq!(s.renew_credential("w-1", 1_070, 60).unwrap(), Some(1_670));
    // a small gain that lands exactly on the cap is still written
    assert_eq!(s.renew_credential("w-1", 1_395, 60).unwrap(), Some(1_995));
    assert_eq!(s.renew_credential("w-1", 1_400, 60).unwrap(), Some(2_000));
}

#[test]
fn a_revoked_expired_unknown_or_legacy_credential_is_never_renewed() {
    let s = store();
    issue_renewable(&s, 1_000, 600, 5_000);
    assert_eq!(s.renew_credential("w-1", 1_600, 60).unwrap(), None);
    assert_eq!(s.renew_credential("nobody", 1_100, 60).unwrap(), None);
    assert!(s.revoke_credential("w-1", 1_200).unwrap());
    assert_eq!(s.renew_credential("w-1", 1_300, 60).unwrap(), None);
    // max_life 0 is a credential that never renews, as one issued before renewal existed
    issue(&s, "w-2", 1_000, 600).unwrap();
    assert_eq!(s.renew_credential("w-2", 1_300, 60).unwrap(), None);
    assert_eq!(s.credential("w-2").unwrap().unwrap().expires, 1_600);
}
