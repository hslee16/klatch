//! R29: the floors (agent cap, label order, scrub and deny list) are not editable by any agent call path.

use super::*;
use crate::gateway::ForbidFloors;

fn agent_gateway() -> Gateway<SpyStore> {
    Gateway::new(
        SpyStore::new(),
        FixedClock(100),
        ForbidFloors::new(AllowAll),
    )
}

/// The floors as stored: the cap, the label order and every scrub rule.
fn floors(g: &Gateway<SpyStore>) -> (Option<String>, Option<String>, Vec<(String, String)>) {
    (
        g.store.policy_get("cap").unwrap(),
        g.store.policy_get("chain").unwrap(),
        g.admin_scrub_terms(&admin()).unwrap(),
    )
}

#[test]
fn no_agent_call_changes_a_floor() {
    let g = agent_gateway();
    let before = floors(&g);
    let attempts = [
        ("cap", g.admin_set_cap(&alice(), "internal")),
        ("chain", g.admin_set_label_chain(&alice(), &["low", "high"])),
        ("scrub-add", g.admin_add_scrub_term(&alice(), "r", "token")),
        ("scrub-remove", g.admin_remove_scrub_term(&alice(), "r")),
    ];
    for (name, result) in attempts {
        assert!(
            matches!(&result, Err(Error::Refused { rule, .. }) if rule == "floors"),
            "{name}: {result:?}"
        );
    }
    assert_eq!(floors(&g), before, "a refused call must change nothing");
}

#[test]
fn a_refused_floor_call_is_not_audited_as_done() {
    let g = agent_gateway();
    let _ = g.admin_set_cap(&alice(), "internal");
    assert!(g.admin_audit(&admin(), 10).unwrap().is_empty());
}

#[test]
fn the_same_floor_calls_work_on_a_gateway_the_human_terminal_opened() {
    // the human path is a gateway without the floor guard (the terminal-gated `klatch admin`)
    let g = open();
    g.admin_set_cap(&admin(), "internal").unwrap();
    g.admin_add_scrub_term(&admin(), "r", "token").unwrap();
    assert_eq!(
        g.store.policy_get("cap").unwrap().as_deref(),
        Some("internal")
    );
}

#[test]
fn an_agent_gateway_still_allows_the_acts_that_are_not_floors() {
    // the admin agent (D4) administers rooms, claims and clearances inside the floors
    let g = agent_gateway();
    g.admin_close_room(&alice(), "r", true).unwrap();
    g.admin_set_room_label(&alice(), "r", "internal").unwrap();
    g.admin_set_clearance(&alice(), "bob", "internal").unwrap();
}

#[test]
fn an_agent_cannot_lift_a_clearance_above_the_cap_either() {
    let g = agent_gateway();
    assert!(matches!(
        g.admin_set_clearance(&alice(), "bob", "secret"),
        Err(Error::Refused { rule, .. }) if rule == "agent-cap"
    ));
}

#[test]
fn the_inner_guard_still_decides_what_the_floor_guard_lets_through() {
    struct DenyAll;
    impl Guard for DenyAll {
        fn check(&self, _: &Principal, _: &Operation) -> Result<()> {
            Err(Error::Refused {
                rule: "inner".into(),
                reason: "no".into(),
            })
        }
    }
    let g = Gateway::new(SpyStore::new(), FixedClock(100), ForbidFloors::new(DenyAll));
    assert!(matches!(
        g.admin_close_room(&alice(), "r", true),
        Err(Error::Refused { rule, .. }) if rule == "inner"
    ));
}
