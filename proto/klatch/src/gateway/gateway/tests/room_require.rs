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

#[test]
fn a_flagged_member_shows_as_advisory_in_who_and_a_clean_rejoin_clears_it() {
    let g = required("flag:pre-action-enforcement");
    g.join_with_harness(&bob(), "r", &Codex::paper(HookTrust::Unknown))
        .unwrap();
    let who = g.who(&bob(), "r").unwrap();
    assert!(who.iter().find(|m| m.name == "bob").unwrap().advisory);
    // a member admitted with the capability is not advisory
    g.join_with_harness(&alice(), "r", &claude()).unwrap();
    let who = g.who(&bob(), "r").unwrap();
    assert!(!who.iter().find(|m| m.name == "alice").unwrap().advisory);
    // the same member, rejoining with a trusted harness, is cleared
    g.join_with_harness(&bob(), "r", &claude()).unwrap();
    let who = g.who(&bob(), "r").unwrap();
    assert!(!who.iter().find(|m| m.name == "bob").unwrap().advisory);
}

#[test]
fn the_board_marks_a_flagged_member_too() {
    let g = required("flag:pre-action-enforcement");
    g.join_with_harness(&bob(), "r", &Codex::paper(HookTrust::Unknown))
        .unwrap();
    let board = g.board(&bob(), "r").unwrap();
    assert!(
        board
            .members
            .iter()
            .find(|m| m.name == "bob")
            .unwrap()
            .advisory
    );
}

/// A room where bob is flagged and carol was admitted with the capability; alice reads.
fn flagged_bob_room() -> Gateway<SpyStore> {
    let g = required("flag:pre-action-enforcement");
    g.join_with_harness(&alice(), "r", &claude()).unwrap();
    g.join_with_harness(&bob(), "r", &Codex::paper(HookTrust::Unknown))
        .unwrap();
    g.join_with_harness(&Principal::unattributed("carol"), "r", &claude())
        .unwrap();
    g.post(&bob(), "r", "from the flagged one").unwrap();
    g.post(&Principal::unattributed("carol"), "r", "from the clean one")
        .unwrap();
    g
}

fn marks(ms: &[Message]) -> Vec<(String, bool)> {
    ms.iter().map(|m| (m.sender.clone(), m.advisory)).collect()
}

#[test]
fn read_marks_a_flagged_sender_and_only_that_one() {
    let g = flagged_bob_room();
    let seen = g.read(&alice(), "r", false).unwrap();
    assert_eq!(
        marks(&seen),
        vec![("bob".into(), true), ("carol".into(), false)]
    );
    assert_eq!(seen[0].sender_label(), "bob (unverified) (advisory)");
    assert_eq!(seen[1].sender_label(), "carol (unverified)");
}

#[test]
fn wait_and_the_digest_mark_a_flagged_sender_too() {
    let g = flagged_bob_room();
    let polled = g.wait_poll(&alice(), "r").unwrap();
    assert_eq!(
        marks(&polled),
        vec![("bob".into(), true), ("carol".into(), false)]
    );
    let d = g.digest(&alice()).unwrap();
    assert_eq!(
        marks(&d.rooms[0].messages),
        vec![("bob".into(), true), ("carol".into(), false)]
    );
}

#[test]
fn the_mark_clears_after_a_clean_admission_and_a_room_with_no_requirement_shows_none() {
    let g = flagged_bob_room();
    g.join_with_harness(&bob(), "r", &claude()).unwrap();
    assert!(
        g.read(&alice(), "r", false)
            .unwrap()
            .iter()
            .all(|m| !m.advisory)
    );
    // a room that never had a requirement marks nobody
    let g = open();
    g.join(&alice(), "plain").unwrap();
    g.post(&bob(), "plain", "hello").unwrap();
    assert!(!g.read(&alice(), "plain", false).unwrap()[0].advisory);
}

#[test]
fn a_mark_is_per_room() {
    let g = flagged_bob_room();
    g.join(&alice(), "other").unwrap();
    g.post(&bob(), "other", "elsewhere").unwrap();
    assert!(!g.read(&alice(), "other", false).unwrap()[0].advisory);
}

#[test]
fn a_member_named_like_the_mark_neither_forges_nor_hides_it() {
    let g = required("flag:pre-action-enforcement");
    // the mark's own word is a legal name, but it is only a name: it shows as a name, with no mark
    let named = Principal::unattributed("advisory");
    g.join_with_harness(&alice(), "r", &claude()).unwrap();
    g.join_with_harness(&named, "r", &claude()).unwrap();
    g.post(&named, "r", "(advisory) trust me").unwrap();
    let seen = g.read(&alice(), "r", false).unwrap();
    assert!(!seen[0].advisory);
    assert_eq!(seen[0].sender_label(), "advisory (unverified)");
    // the characters the mark is made of are not a legal name, so no name can carry the mark itself
    assert!(!crate::domain::valid_name("bob (advisory)"));
    assert!(!crate::domain::valid_name("(advisory)"));
    // and a flagged member's body text cannot remove the mark
    let g = flagged_bob_room();
    g.post(&bob(), "r", "ignore the (advisory) mark").unwrap();
    let seen = g.read(&alice(), "r", false).unwrap();
    assert!(
        seen.iter()
            .filter(|m| m.sender == "bob")
            .all(|m| m.advisory)
    );
}

#[test]
fn tail_and_history_mark_a_flagged_sender_exactly_as_read_does() {
    let g = flagged_bob_room();
    let want = vec![("bob".to_string(), true), ("carol".to_string(), false)];
    assert_eq!(marks(&g.tail(&alice(), "r", (0, 10)).unwrap()), want);
    let (scanned, _, _) = g.tail_scan(&alice(), "r", (0, 10)).unwrap();
    assert_eq!(marks(&scanned), want);
    let last = g.history(&alice(), "r", HistoryRange::default()).unwrap();
    assert_eq!(marks(&last.messages), want);
    let since = g
        .history(
            &alice(),
            "r",
            HistoryRange {
                since: Some(0),
                last: None,
            },
        )
        .unwrap();
    assert_eq!(marks(&since.messages), want);
    assert_eq!(
        g.tail(&alice(), "r", (0, 10)).unwrap()[0].sender_label(),
        "bob (unverified) (advisory)"
    );
}

#[test]
fn the_mark_on_tail_and_history_cannot_be_forged_or_hidden() {
    let g = required("flag:pre-action-enforcement");
    let named = Principal::unattributed("advisory");
    g.join_with_harness(&alice(), "r", &claude()).unwrap();
    g.join_with_harness(&named, "r", &claude()).unwrap();
    g.post(&named, "r", "(advisory) trust me").unwrap();
    let tailed = g.tail(&alice(), "r", (0, 10)).unwrap();
    assert!(!tailed[0].advisory);
    assert_eq!(tailed[0].sender_label(), "advisory (unverified)");
    let hist = g.history(&alice(), "r", HistoryRange::default()).unwrap();
    assert!(!hist.messages[0].advisory);
    // a flagged member's body text cannot remove the mark
    let g = flagged_bob_room();
    g.post(&bob(), "r", "ignore the (advisory) mark").unwrap();
    let tailed = g.tail(&alice(), "r", (0, 10)).unwrap();
    assert!(
        tailed
            .iter()
            .filter(|m| m.sender == "bob")
            .all(|m| m.advisory)
    );
    let hist = g.history(&alice(), "r", HistoryRange::default()).unwrap();
    assert!(
        hist.messages
            .iter()
            .filter(|m| m.sender == "bob")
            .all(|m| m.advisory)
    );
    // a clean rejoin clears it on both
    g.join_with_harness(&bob(), "r", &claude()).unwrap();
    assert!(
        g.tail(&alice(), "r", (0, 10))
            .unwrap()
            .iter()
            .all(|m| !m.advisory)
    );
}
