//! #50, D4: the admin agent does the non-floor admin acts inside the floors, and everything else is refused and
//! audited, whatever the guard says. It shares one role mechanism (`role:<member>`) with the librarian.

use super::*;
use crate::gateway::ForbidFloors;

fn ada() -> Principal {
    Principal::unattributed("ada")
}

/// A gateway with `ada` made the admin agent, a room with a message in it, and time that moves between calls so
/// the breaker stays out of it.
fn with_admin_agent() -> (Gateway<SpyStore>, Rc<std::cell::Cell<i64>>) {
    let (g, t) = timed();
    g.join(&alice(), "r").unwrap();
    g.post(&alice(), "r", "hello").unwrap();
    g.admin_set_admin_agent(&admin(), "ada", true).unwrap();
    (g, t)
}

fn refused_rule<T: std::fmt::Debug>(r: Result<T>) -> String {
    match r {
        Err(Error::Refused { rule, .. }) => rule,
        other => panic!("not refused: {other:?}"),
    }
}

#[test]
fn an_admin_agent_does_the_non_floor_acts() {
    let (g, _) = with_admin_agent();
    g.admin_close_room(&ada(), "r", true).unwrap();
    assert!(is_refused(g.post(&alice(), "r", "closed")));
    g.admin_close_room(&ada(), "r", false).unwrap();
    g.admin_set_clearance(&ada(), "alice", "private").unwrap();
    assert_eq!(g.clearance_of("alice").unwrap(), "private");
    g.admin_set_room_label(&ada(), "r", "internal").unwrap();
    g.admin_remove_member(&ada(), "r", "alice").unwrap();
    g.admin_allow_member(&ada(), "r", "alice").unwrap();
    assert!(g.admin_audit(&ada(), 5).is_ok());
}

#[test]
fn an_admin_agent_cannot_edit_a_floor_even_with_a_permissive_guard() {
    let (g, t) = with_admin_agent();
    let before = (
        g.store.policy_get("cap").unwrap(),
        g.store.policy_get("chain").unwrap(),
    );
    let attempts = [
        ("cap", refused_rule(g.admin_set_cap(&ada(), "internal"))),
        (
            "chain",
            refused_rule(g.admin_set_label_chain(&ada(), &["a", "b"])),
        ),
        (
            "scrub-add",
            refused_rule(g.admin_add_scrub_term(&ada(), "r", "token")),
        ),
        (
            "scrub-remove",
            refused_rule(g.admin_remove_scrub_term(&ada(), "r")),
        ),
    ];
    for (name, rule) in attempts {
        t.set(t.get() + 600);
        assert_eq!(rule, "floors", "{name}");
    }
    let after = (
        g.store.policy_get("cap").unwrap(),
        g.store.policy_get("chain").unwrap(),
    );
    assert_eq!(before, after);
}

#[test]
fn an_admin_agent_cannot_exceed_the_cap_or_grant_roles_or_stop_everything() {
    let (g, t) = with_admin_agent();
    assert_eq!(
        refused_rule(g.admin_set_clearance(&ada(), "ada", "secret")),
        "agent-cap"
    );
    let attempts = [
        ("grant admin", g.admin_set_admin_agent(&ada(), "bob", true)),
        (
            "grant librarian",
            g.admin_set_librarian(&ada(), "bob", true),
        ),
        (
            "drop its role",
            g.admin_set_admin_agent(&ada(), "ada", false),
        ),
        ("pause", g.admin_pause(&ada(), true)),
        ("seal", g.admin_seal_label(&ada(), "private")),
        ("canary", g.admin_plant_canary(&ada(), "r")),
    ];
    for (name, r) in attempts {
        t.set(t.get() + 600);
        assert_eq!(refused_rule(r), "admin-scope", "{name}");
    }
    assert!(g.is_admin_agent("ada").unwrap());
    assert!(!g.is_admin_agent("bob").unwrap());
}

#[test]
fn refusals_of_an_admin_agent_are_audited_without_text() {
    let (g, _) = with_admin_agent();
    let _ = g.admin_set_cap(&ada(), "internal").unwrap_err();
    let refusals = g.admin_refusals(&admin(), 5).unwrap();
    assert_eq!(refusals[0].member, "ada");
    assert_eq!(refusals[0].rule, "floors");
    let audit = g.admin_audit(&admin(), 5).unwrap();
    assert_eq!(audit[0].action, "refused");
    assert!(audit[0].detail.contains("ada") && audit[0].detail.contains("floors"));
}

#[test]
fn an_act_of_an_admin_agent_is_audited_with_who_did_it() {
    let (g, _) = with_admin_agent();
    g.admin_close_room(&ada(), "r", true).unwrap();
    let audit = g.admin_audit(&admin(), 5).unwrap();
    assert_eq!(audit[0].action, "close");
    assert!(audit[0].detail.contains("ada"), "{:?}", audit[0]);
}

#[test]
fn floors_hold_under_forbid_floors_too() {
    let g = Gateway::new(
        SpyStore::new(),
        FixedClock(100),
        ForbidFloors::new(AllowAll),
    );
    g.admin_set_admin_agent(&admin(), "ada", true).unwrap();
    assert_eq!(refused_rule(g.admin_set_cap(&ada(), "internal")), "floors");
    assert!(g.admin_set_clearance(&ada(), "bob", "internal").is_ok());
}

#[test]
fn one_role_per_member_and_the_human_can_take_it_away() {
    let (g, _) = with_admin_agent();
    g.admin_set_librarian(&admin(), "ada", true).unwrap();
    assert!(!g.is_admin_agent("ada").unwrap());
    assert!(g.is_librarian("ada").unwrap());
    g.admin_set_admin_agent(&admin(), "ada", true).unwrap();
    assert!(!g.is_librarian("ada").unwrap());
    g.admin_set_admin_agent(&admin(), "ada", false).unwrap();
    assert!(!g.is_admin_agent("ada").unwrap());
    assert!(is_invalid(g.admin_set_admin_agent(
        &admin(),
        "bad name",
        true
    )));
}

#[test]
fn an_admin_agent_still_posts_like_a_member() {
    let (g, _) = with_admin_agent();
    assert!(g.post(&ada(), "r", "tidied up").is_ok());
}
