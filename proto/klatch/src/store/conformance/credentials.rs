use super::*;
use crate::store::NewCredential;
fn hash(b: u8) -> [u8; 32] {
    [b; 32]
}
fn issue<S: Store>(s: &S, member: &str, now: i64, ttl: i64) -> Result<()> {
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
pub fn an_issued_credential_is_stored_with_its_hash_clearance_parent_and_expiry<S: Store>(
    make: fn() -> S,
) {
    let s = make();
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
pub fn revoking_works_once_and_marks_the_record_without_deleting_it<S: Store>(make: fn() -> S) {
    let s = make();
    issue(&s, "w-1", 1_000, 600).unwrap();
    assert!(s.revoke_credential("w-1", 1_100).unwrap());
    assert!(!s.revoke_credential("w-1", 1_200).unwrap());
    assert!(!s.revoke_credential("never-issued", 1_200).unwrap());
    assert!(s.credential("w-1").unwrap().unwrap().revoked);
}
pub fn the_list_has_every_credential_by_member<S: Store>(make: fn() -> S) {
    let s = make();
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
