//! #61: a room can require a harness capability. The human sets `room:ROOM:require` (admin only, audited); a join
//! from a harness that lacks one is refused or flagged by the room's mode, and a requirement this build cannot read
//! refuses every joiner (fail closed). A caller that names no harness is the generic floor.

use super::*;
use crate::harness::{Admission, Support};
use crate::harness::{ClaudeCode, Codex, HookTrust};

fn claude() -> ClaudeCode {
    ClaudeCode::new(Support::Unknown(String::new()))
}

fn required(spec: &str) -> Gateway<SpyStore> {
    let g = open();
    g.admin_set_room_require(&admin(), "r", Some(spec)).unwrap();
    g
}

#[test]
fn a_room_with_no_requirement_admits_anyone() {
    let g = open();
    assert_eq!(g.room_requirement("r").unwrap(), None);
    g.join(&alice(), "r").unwrap();
    assert_eq!(
        g.join_with_harness(&bob(), "r", &Codex::paper(HookTrust::Unknown))
            .unwrap(),
        Admission::Admit
    );
}

#[test]
fn the_human_sets_a_requirement_and_it_is_stored_and_audited() {
    let g = required("refuse:pre-action-enforcement");
    assert_eq!(
        g.store_for_tests().policy_get("room:r:require").unwrap(),
        Some("refuse:pre-action-enforcement".into())
    );
    assert!(
        g.admin_audit(&admin(), 10)
            .unwrap()
            .iter()
            .any(|r| r.action == "room-require" && r.target == "r refuse:pre-action-enforcement")
    );
}

#[test]
fn off_removes_the_requirement() {
    let g = required("refuse:wake");
    g.admin_set_room_require(&admin(), "r", None).unwrap();
    assert_eq!(g.room_requirement("r").unwrap(), None);
    assert_eq!(
        g.store_for_tests().policy_get("room:r:require").unwrap(),
        None
    );
    g.join(&alice(), "r").unwrap();
}

#[test]
fn an_unknown_capability_or_a_malformed_spec_is_refused_at_set_time() {
    let g = open();
    for bad in ["refuse:telepathy", "wake", "maybe:wake", "refuse:", ""] {
        assert!(
            is_invalid(g.admin_set_room_require(&admin(), "r", Some(bad))),
            "{bad}"
        );
    }
    assert!(is_invalid(g.admin_set_room_require(
        &admin(),
        "bad room!",
        Some("flag:wake")
    )));
    assert_eq!(g.room_requirement("r").unwrap(), None);
}

#[test]
fn only_the_human_sets_a_requirement_not_an_admin_agent_or_a_member() {
    let g = open();
    g.admin_set_admin_agent(&admin(), "ada", true).unwrap();
    assert_eq!(
        rule_of(
            g.admin_set_room_require(&Principal::unattributed("ada"), "r", Some("flag:wake"))
                .unwrap_err()
        ),
        "admin-scope"
    );
    let members = open().for_members();
    assert!(is_refused(members.admin_set_room_require(
        &alice(),
        "r",
        Some("flag:wake")
    )));
    assert_eq!(g.room_requirement("r").unwrap(), None);
}

#[test]
fn a_harness_with_the_capability_joins_and_one_without_is_refused() {
    let g = required("refuse:pre-action-enforcement");
    assert_eq!(
        g.join_with_harness(&alice(), "r", &claude()).unwrap(),
        Admission::Admit
    );
    let e = g
        .join_with_harness(&bob(), "r", &Codex::paper(HookTrust::Untrusted))
        .unwrap_err();
    assert_eq!(rule_of(e), "harness-capability");
    assert!(
        !g.who(&alice(), "r")
            .unwrap()
            .iter()
            .any(|m| m.name == "bob")
    );
}

#[test]
fn a_refused_join_leaves_a_refusal_record_naming_the_rule_and_never_the_harness_text() {
    let g = required("refuse:wake");
    let _ = g.join_with_harness(&bob(), "r", &Codex::paper(HookTrust::Trusted));
    let rows = g.admin_refusals(&admin(), 10).unwrap();
    assert!(
        rows.iter()
            .any(|r| r.rule == "harness-capability" && r.operation == "join")
    );
}

#[test]
fn a_flag_mode_room_admits_the_member_and_records_the_gap() {
    let g = required("flag:pre-action-enforcement");
    let a = g
        .join_with_harness(&bob(), "r", &Codex::paper(HookTrust::Unknown))
        .unwrap();
    assert!(matches!(a, Admission::Flag(_)));
    assert!(g.who(&bob(), "r").unwrap().iter().any(|m| m.name == "bob"));
    let rows = g.admin_refusals(&admin(), 10).unwrap();
    assert!(
        rows.iter()
            .any(|r| r.rule == "harness-flagged" && r.operation == "join")
    );
}

#[test]
fn a_plain_join_is_the_generic_floor_so_a_requiring_room_refuses_it() {
    let g = required("refuse:pre-action-enforcement");
    assert_eq!(
        rule_of(g.join(&alice(), "r").unwrap_err()),
        "harness-capability"
    );
    let f = required("flag:wake");
    f.join(&alice(), "r").unwrap();
}

#[test]
fn a_requirement_this_build_cannot_read_refuses_every_joiner_even_in_flag_mode() {
    for stored in ["flag:telepathy", "garbage", "refuse:", "flag:wake,wake"] {
        let g = open();
        g.store_for_tests()
            .policy_set("room:r:require", Some(stored), 1)
            .unwrap();
        assert!(g.room_requirement("r").is_err(), "{stored}");
        assert_eq!(
            rule_of(g.join_with_harness(&alice(), "r", &claude()).unwrap_err()),
            "harness-capability",
            "{stored}"
        );
        assert_eq!(
            rule_of(g.join(&alice(), "r").unwrap_err()),
            "harness-capability"
        );
    }
}

#[test]
fn a_requirement_binds_only_its_own_room() {
    let g = required("refuse:wake");
    g.join(&alice(), "other").unwrap();
}
