//! Issued credentials (#121, #120, #161): a launcher gets a token for a worker; the token proves the member, expires
//! and can be revoked; a verified message means the sender held a live credential.

use super::*;
use klatch_core::Attribution;

fn spec(member: &str) -> CredentialSpec<'_> {
    CredentialSpec {
        member,
        clearance: "internal",
        ttl: 600,
        parent: Some("lead"),
    }
}

#[test]
fn an_issued_token_proves_its_member_as_a_verified_principal_with_the_clearance_it_carries() {
    let g = open();
    let token = g.admin_issue_credential(&admin(), &spec("w-1")).unwrap();
    let who = g.verify_credential("w-1", &token).unwrap();
    assert_eq!(
        (who.member.as_str(), who.attribution),
        ("w-1", Attribution::Verified)
    );
    assert_eq!(g.clearance_of("w-1").unwrap(), "internal");
    assert_eq!(g.store.parent_of("w-1").unwrap().as_deref(), Some("lead"));
}

#[test]
fn a_message_from_a_verified_principal_is_stored_as_verified() {
    let g = open();
    let token = g.admin_issue_credential(&admin(), &spec("w-1")).unwrap();
    let who = g.verify_credential("w-1", &token).unwrap();
    g.post(&who, "r", "from a worker").unwrap();
    g.post(&bob(), "r", "from a typed name").unwrap();
    let m = g.tail(&alice(), "r", (0, 10)).unwrap();
    assert_eq!(m[0].attribution, Attribution::Verified);
    assert_eq!(m[1].attribution, Attribution::Unattributed);
}

#[test]
fn the_audit_records_the_issue_with_the_parent_and_never_the_token() {
    let g = open();
    let token = g.admin_issue_credential(&admin(), &spec("w-1")).unwrap();
    let audit = g.admin_audit(&admin(), 5).unwrap();
    let row = audit
        .iter()
        .find(|a| a.action == "issue-credential")
        .expect("no audit row");
    assert!(
        row.target == "w-1" && row.detail.contains("lead") && row.detail.contains("internal"),
        "{row:?}"
    );
    let text = token.to_text();
    assert!(
        audit
            .iter()
            .all(|a| !a.detail.contains(text.as_str()) && !a.target.contains(text.as_str()))
    );
}

#[test]
fn a_wrong_token_an_unknown_member_a_revoked_and_an_expired_credential_are_all_refused() {
    let (g, t) = timed();
    let token = g.admin_issue_credential(&admin(), &spec("w-1")).unwrap();
    let other = klatch_keys::Token::generate().unwrap();
    assert_eq!(
        rule_of(g.verify_credential("w-1", &other).unwrap_err()),
        "credential"
    );
    assert_eq!(
        rule_of(g.verify_credential("ghost", &token).unwrap_err()),
        "credential"
    );
    // a token cannot be used for another member
    g.admin_issue_credential(&admin(), &spec("w-2")).unwrap();
    assert_eq!(
        rule_of(g.verify_credential("w-2", &token).unwrap_err()),
        "credential"
    );
    // expiry
    t.set(t.get() + 601);
    let e = g.verify_credential("w-1", &token).unwrap_err().to_string();
    assert!(e.contains("expired"), "{e}");
    // revocation
    let fresh = g.admin_issue_credential(&admin(), &spec("w-1")).unwrap();
    g.verify_credential("w-1", &fresh).unwrap();
    g.admin_revoke_credential(&admin(), "w-1").unwrap();
    assert!(
        g.verify_credential("w-1", &fresh)
            .unwrap_err()
            .to_string()
            .contains("revoked")
    );
}

#[test]
fn reissuing_replaces_the_credential_and_the_old_token_stops_working() {
    let (g, t) = timed();
    let old = g.admin_issue_credential(&admin(), &spec("w-1")).unwrap();
    assert!(
        is_invalid(g.admin_issue_credential(&admin(), &spec("w-1"))),
        "a live credential was replaced"
    );
    g.admin_revoke_credential(&admin(), "w-1").unwrap();
    t.set(t.get() + 1);
    let new = g.admin_issue_credential(&admin(), &spec("w-1")).unwrap();
    assert!(g.verify_credential("w-1", &old).is_err());
    g.verify_credential("w-1", &new).unwrap();
}

#[test]
fn the_clearance_cannot_pass_the_agent_cap_nor_the_lifetime_the_limit() {
    let g = open();
    let mut s = spec("w-1");
    s.clearance = "secret";
    assert_eq!(
        rule_of(g.admin_issue_credential(&admin(), &s).unwrap_err()),
        "agent-cap"
    );
    for ttl in [0, -5, 31 * 24 * 3600] {
        let mut s = spec("w-1");
        s.ttl = ttl;
        assert!(
            is_invalid(g.admin_issue_credential(&admin(), &s)),
            "ttl {ttl}"
        );
    }
    let mut s = spec("w-1");
    s.clearance = "no-such-label";
    assert!(is_invalid(g.admin_issue_credential(&admin(), &s)));
}

#[test]
fn names_that_would_impersonate_the_system_or_the_launcher_are_refused() {
    let g = open();
    for bad in ["bad name", "", "admin", "canary"] {
        assert!(
            is_invalid(g.admin_issue_credential(&admin(), &spec(bad))),
            "{bad:?}"
        );
    }
    let mut s = spec("w-1");
    s.parent = Some("w-1");
    assert!(
        is_invalid(g.admin_issue_credential(&admin(), &s)),
        "a worker is its own parent"
    );
    s.parent = Some("bad parent");
    assert!(is_invalid(g.admin_issue_credential(&admin(), &s)));
}

#[test]
fn a_failed_verification_is_in_the_refusal_record_and_never_holds_the_token() {
    let g = open();
    let token = g.admin_issue_credential(&admin(), &spec("w-1")).unwrap();
    let other = klatch_keys::Token::generate().unwrap();
    let _ = g.verify_credential("w-1", &other);
    let refusals = g.admin_refusals(&admin(), 5).unwrap();
    assert!(
        refusals
            .iter()
            .any(|r| r.rule == "credential" && r.member == "w-1"),
        "{refusals:?}"
    );
    let all = format!("{refusals:?}");
    assert!(!all.contains(other.to_text().as_str()) && !all.contains(token.to_text().as_str()));
}

#[test]
fn a_worker_cannot_enter_a_room_above_the_clearance_its_credential_carries() {
    let g = open();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    let token = g.admin_issue_credential(&admin(), &spec("w-1")).unwrap();
    let who = g.verify_credential("w-1", &token).unwrap();
    assert!(is_refused(g.join(&who, "vault")));
    g.join(&who, "open-room").unwrap();
}

#[test]
fn credentials_are_listed_without_their_hashes_and_say_their_state() {
    let (g, t) = timed();
    g.admin_issue_credential(&admin(), &spec("w-1")).unwrap();
    g.admin_issue_credential(&admin(), &spec("w-2")).unwrap();
    g.admin_revoke_credential(&admin(), "w-2").unwrap();
    t.set(t.get() + 700);
    let list = g.admin_credentials(&admin()).unwrap();
    let state = |m: &str| list.iter().find(|c| c.member == m).unwrap().state;
    assert_eq!((state("w-1"), state("w-2")), ("expired", "revoked"));
    assert!(g.admin_revoke_credential(&admin(), "nobody").is_err());
}
