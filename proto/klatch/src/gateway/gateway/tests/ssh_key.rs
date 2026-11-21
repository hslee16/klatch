//! #107: an ssh forced-command key label is mapped to one member by the human at a terminal. The mapping is a floor
//! act (no agent, the admin agent included), stored in the policy table, audited, and the identity built from it gives
//! a verified member for a mapped label and an unattributed one for anything else.

use super::*;
use crate::gateway::{ForbidFloors, Identity, PeerEvidence, SshKeyIdentity, ssh_key_member};

fn forced(label: &str) -> PeerEvidence {
    PeerEvidence::SshForcedKey {
        label: label.into(),
    }
}

#[test]
fn the_human_maps_a_label_to_one_member_and_the_act_is_audited() {
    let g = open();
    g.admin_set_ssh_key(&admin(), "laptop", "alice-remote", true)
        .unwrap();
    assert_eq!(
        ssh_key_member(&g.store, "laptop").unwrap().as_deref(),
        Some("alice-remote")
    );
    let log = g.admin_audit(&admin(), 10).unwrap();
    let row = log.iter().find(|a| a.action == "ssh-key").expect("audited");
    assert_eq!(row.target, "laptop/alice-remote=true");
}

#[test]
fn no_agent_can_map_a_key_and_nothing_is_stored() {
    // every gateway an agent can reach is wrapped in ForbidFloors
    let agent = Gateway::new(
        SpyStore::new(),
        FixedClock(100),
        ForbidFloors::new(AllowAll),
    );
    let e = agent
        .admin_set_ssh_key(&alice(), "laptop", "alice", true)
        .unwrap_err();
    assert_eq!(rule_of(e), "floors");
    assert_eq!(ssh_key_member(&agent.store, "laptop").unwrap(), None);
    // the admin agent role does not reach it either, on a members' gateway
    let g = open();
    g.admin_set_admin_agent(&admin(), "ada", true).unwrap();
    let members = g.for_members();
    let ada = Principal::unattributed("ada");
    let e = members
        .admin_set_ssh_key(&ada, "laptop", "ada", true)
        .unwrap_err();
    assert_eq!(rule_of(e), "floors");
    assert_eq!(ssh_key_member(&members.store, "laptop").unwrap(), None);
}

#[test]
fn a_label_maps_to_one_member_and_a_remap_needs_an_explicit_unmap() {
    let g = open();
    g.admin_set_ssh_key(&admin(), "laptop", "alice", true)
        .unwrap();
    // naming it again for the same member changes nothing and is allowed
    g.admin_set_ssh_key(&admin(), "laptop", "alice", true)
        .unwrap();
    // another member is refused, and the refusal is kept
    let e = g
        .admin_set_ssh_key(&admin(), "laptop", "bob", true)
        .unwrap_err();
    assert_eq!(rule_of(e), "ssh-key-mapped");
    assert_eq!(
        ssh_key_member(&g.store, "laptop").unwrap().as_deref(),
        Some("alice")
    );
    // unmapping names the member it is taken from
    assert!(is_invalid(g.admin_set_ssh_key(
        &admin(),
        "laptop",
        "bob",
        false
    )));
    g.admin_set_ssh_key(&admin(), "laptop", "alice", false)
        .unwrap();
    assert_eq!(ssh_key_member(&g.store, "laptop").unwrap(), None);
    g.admin_set_ssh_key(&admin(), "laptop", "bob", true)
        .unwrap();
    assert_eq!(
        ssh_key_member(&g.store, "laptop").unwrap().as_deref(),
        Some("bob")
    );
}

#[test]
fn a_bad_label_or_member_or_a_reserved_name_is_refused_and_stores_nothing() {
    let g = open();
    let long = "x".repeat(41);
    for (label, member) in [
        ("", "alice"),
        ("has space", "alice"),
        ("quote\"d", "alice"),
        ("semi;colon", "alice"),
        (long.as_str(), "alice"),
        ("ok", ""),
        ("ok", "has space"),
        ("ok", "admin"),
    ] {
        assert!(
            is_invalid(g.admin_set_ssh_key(&admin(), label, member, true)),
            "{label:?} {member:?}"
        );
    }
    assert!(g.store.policy_list("ssh-key:").unwrap().is_empty());
}

#[test]
fn two_labels_resolve_to_two_members_and_neither_is_the_other() {
    let g = open();
    g.admin_set_ssh_key(&admin(), "a", "member-a", true)
        .unwrap();
    g.admin_set_ssh_key(&admin(), "b", "member-b", true)
        .unwrap();
    let id = SshKeyIdentity::new(&g.store);
    let a = id.resolve(&forced("a")).unwrap();
    let b = id.resolve(&forced("b")).unwrap();
    assert_eq!(
        (a.member.as_str(), b.member.as_str()),
        ("member-a", "member-b")
    );
    assert_eq!(a.attribution, crate::domain::Attribution::Verified);
}

#[test]
fn an_unmapped_label_is_unattributed_and_typed_names_are_never_evidence() {
    let g = open();
    g.admin_set_ssh_key(&admin(), "a", "member-a", true)
        .unwrap();
    let id = SshKeyIdentity::new(&g.store);
    let p = id.resolve(&forced("nobody")).unwrap();
    assert_eq!(p.attribution, crate::domain::Attribution::Unattributed);
    assert_ne!(p.member, "member-a");
    // a name the peer typed, a bearer token, nothing at all: this identity does not take them
    for ev in [
        PeerEvidence::DeclaredName("member-a".into()),
        PeerEvidence::bearer("member-a"),
        PeerEvidence::None,
    ] {
        assert!(id.resolve(&ev).is_err(), "{ev:?}");
    }
}

#[test]
fn the_evidence_names_its_kind_and_not_the_label() {
    assert_eq!(forced("secret-looking-label").kind(), "ssh-forced");
}
