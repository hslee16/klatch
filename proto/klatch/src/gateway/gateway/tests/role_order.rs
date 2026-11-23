//! #1444: the order of the checks in `check_role`, pinned before it was split into steps. A caller that trips two of
//! them gets the refusal of the earlier one. Pinned here: reserved name before reviewer; reviewer before the merge
//! claim; the facilitator bound before the merge claim and before the need-info state rule; the merge claim before
//! the librarian's read-only limit; the member admin rule before the admin agent's list; the room capability before
//! the role's own limit. Not pinned: see the design note. These are characterisation tests of
//! a pure refactor: they passed on the code before the split and pass after it.

use super::*;
use crate::domain::DM_PREFIX;

/// A members' gateway whose roles were granted by the human before it was marked for members.
fn members(grant: impl FnOnce(&Gateway<SpyStore>)) -> Gateway<SpyStore> {
    let g = open();
    grant(&g);
    g.for_members()
}

fn person(name: &str) -> Principal {
    Principal::unattributed(name)
}

fn claim(path: &str) -> Operation {
    Operation::Claim {
        room: "lobby".into(),
        path: path.into(),
    }
}

fn post(room: &str, body: &str) -> Operation {
    Operation::Post {
        room: room.into(),
        body: body.into(),
    }
}

fn refusal_of(g: &Gateway<SpyStore>, who: &str, op: &Operation) -> (String, String) {
    match g.check_role(&person(who), op).unwrap_err() {
        Error::Refused { rule, reason } => (rule, reason),
        other => panic!("not a refusal: {other}"),
    }
}

fn rule(g: &Gateway<SpyStore>, who: &str, op: &Operation) -> String {
    refusal_of(g, who, op).0
}

#[test]
fn a_reserved_name_is_refused_before_any_role_is_read() {
    let g = members(|_| {});
    for op in [
        post("lobby", "hi"),
        claim("merge:main"),
        Operation::ReviewNext,
    ] {
        assert_eq!(rule(&g, "admin", &op), "reserved-name");
    }
    // not on the launcher's gateway, where the human acts as `admin`
    assert!(open().check_role(&admin(), &post("lobby", "hi")).is_ok());
}

#[test]
fn a_reviewer_is_refused_as_a_reviewer_before_the_merge_claim_rule() {
    let g = members(|g| g.admin_set_reviewer(&admin(), "rex", true).unwrap());
    assert_eq!(rule(&g, "rex", &claim("merge:main")), "reviewer");
    assert_eq!(rule(&g, "rex", &post("lobby", "hi")), "reviewer");
    assert!(g.check_role(&person("rex"), &Operation::ReviewNext).is_ok());
    // a caller with no role may not review
    let (rule, reason) = refusal_of(&g, "bob", &Operation::ReviewNext);
    assert_eq!(rule, "reviewer");
    assert_eq!(reason, "bob is not a reviewer");
}

#[test]
fn a_facilitator_is_bound_before_the_merge_claim_rule_and_the_need_info_state_rule() {
    let g = members(|g| g.admin_set_facilitator(&admin(), "fay", true).unwrap());
    assert_eq!(rule(&g, "fay", &claim("merge:main")), "facilitator");
    assert_eq!(rule(&g, "fay", &post("lobby", "hi")), "facilitator");
    // a need-info form allowed by the bound, but outside a direct message room it is refused by the state rule
    let dm = format!("{DM_PREFIX}fay.zed");
    let (rule, reason) = refusal_of(&g, "fay", &post(&dm, "need-info #3: which-repo"));
    assert_eq!(rule, "facilitator");
    assert!(reason.ends_with("a need-info message outside a direct message room is outside that"));
    // an ordinary member is not held to the facilitator bound
    assert!(g.check_role(&person("bob"), &post("lobby", "hi")).is_ok());
}

#[test]
fn the_merge_claim_rule_comes_before_the_librarians_read_only_rule() {
    let g = members(|g| {
        g.admin_set_librarian(&admin(), "lia", true).unwrap();
        g.admin_set_integrator(&admin(), "ivy", true).unwrap();
    });
    assert_eq!(rule(&g, "lia", &claim("merge:main")), "integrator");
    assert_eq!(rule(&g, "lia", &post("lobby", "hi")), "read-only");
    assert_eq!(rule(&g, "bob", &claim("merge:main")), "integrator");
    assert!(g.check_role(&person("lia"), &claim_query()).is_ok());
    assert!(g.check_role(&person("ivy"), &claim("merge:main")).is_ok());
    // the human takes it too, and any other claim is no business of this rule
    assert!(open().check_role(&admin(), &claim("merge:main")).is_ok());
    assert!(g.check_role(&person("bob"), &claim("src/a.rs")).is_ok());
}

fn claim_query() -> Operation {
    Operation::Claims {
        room: "lobby".into(),
    }
}

#[test]
fn the_member_admin_rule_comes_before_the_admin_agents_own_list() {
    let g = members(|g| g.admin_set_admin_agent(&admin(), "ada", true).unwrap());
    // a plain member: a floor act is a floor, any other admin act is the admin agent's
    let (rule_floor, _) = refusal_of(&g, "bob", &Operation::admin("cap", ""));
    assert_eq!(rule_floor, "floors");
    let (rule_scope, reason) = refusal_of(&g, "bob", &Operation::admin("audit", ""));
    assert_eq!(rule_scope, "admin-scope");
    assert_eq!(reason, "that act is for the admin agent, not for a member");
    // the open acts of every member
    for act in ["prune", "hook_runs", "label-order", "scrub-check"] {
        assert!(
            g.check_role(&person("bob"), &Operation::admin(act, ""))
                .is_ok(),
            "{act}"
        );
    }
    // the admin agent: its list, and the floors stay the human's
    assert!(
        g.check_role(&person("ada"), &Operation::admin("audit", ""))
            .is_ok()
    );
    assert_eq!(rule(&g, "ada", &Operation::admin("cap", "")), "floors");
    let (rule_out, reason) = refusal_of(&g, "ada", &Operation::admin("no-such-act", ""));
    assert_eq!(rule_out, "admin-scope");
    assert_eq!(reason, "that act is outside the admin agent's role");
    // a gateway not built for members leaves the member admin rule out, and the admin agent's list still applies
    let o = open();
    o.admin_set_admin_agent(&admin(), "ada", true).unwrap();
    assert!(
        o.check_role(&person("bob"), &Operation::admin("audit", ""))
            .is_ok()
    );
    assert_eq!(
        rule(&o, "ada", &Operation::admin("no-such-act", "")),
        "admin-scope"
    );
}

#[test]
fn the_room_capability_comes_before_the_librarians_read_only_limit() {
    // rooms are granted only and the librarian was not granted the capability: creating a room trips both the
    // capability and the read-only limit, and the capability is refused first
    let g = members(|g| {
        g.admin_set_librarian(&admin(), "lia", true).unwrap();
        g.admin_set_room_creation_granted(&admin(), true).unwrap();
    });
    let create = Operation::CreateRoom {
        room: "fresh".into(),
    };
    assert_eq!(rule(&g, "lia", &create), "room-capability");
    let invite = Operation::Invite {
        member: "bob".into(),
        room: "fresh".into(),
    };
    assert_eq!(rule(&g, "lia", &invite), "room-capability");
    // with the capability the read-only limit is what refuses
    let g = members(|g| {
        g.admin_set_librarian(&admin(), "lia", true).unwrap();
        g.admin_set_room_creation_granted(&admin(), true).unwrap();
        g.admin_grant_room_capability(&admin(), "lia", true)
            .unwrap();
    });
    assert_eq!(rule(&g, "lia", &create), "read-only");
}
