//! #507: a gateway built for members (what `klatch act`, `klatch review`, `serve` and the hooks use) refuses an
//! admin act to a member that does not hold the admin agent role. The terminal and launcher gateways are not built
//! this way, so the human's acts are unchanged.

use super::*;

fn member(name: &str) -> Principal {
    Principal::unattributed(name)
}

/// A member-side gateway with a room, `ada` as the admin agent and `lib` as the librarian.
fn members_gateway() -> Gateway<SpyStore> {
    // the human grants the roles through the terminal gateway; members then act through this one
    let (g, _) = timed();
    g.join(&alice(), "r").unwrap();
    g.post(&alice(), "r", "hello").unwrap();
    g.admin_set_admin_agent(&admin(), "ada", true).unwrap();
    g.admin_set_librarian(&admin(), "lib", true).unwrap();
    g.for_members()
}

#[test]
fn an_ordinary_member_is_refused_every_admin_act() {
    let g = members_gateway();
    let me = alice();
    let refused = [
        g.admin_close_room(&me, "r", true).err(),
        g.admin_remove_member(&me, "r", "bob").err(),
        g.admin_allow_member(&me, "r", "bob").err(),
        g.admin_set_clearance(&me, "bob", "internal").err(),
        g.admin_set_room_label(&me, "r", "internal").err(),
        g.admin_audit(&me, 5).err(),
        g.admin_refusals(&me, 5).err(),
        g.admin_pause(&me, true).err(),
        g.admin_set_cap(&me, "internal").err(),
    ];
    for e in refused {
        let rule = rule_of(e.expect("an ordinary member must be refused"));
        assert!(rule == "admin-scope" || rule == "floors", "{rule}");
    }
    assert!(
        g.post(&bob(), "r", "still open").is_ok(),
        "the room stayed open"
    );
    assert_eq!(g.clearance_of("bob").unwrap(), "public");
}

#[test]
fn a_floor_act_by_an_ordinary_member_is_refused_as_a_floor() {
    let g = members_gateway();
    assert_eq!(
        rule_of(g.admin_set_cap(&alice(), "internal").unwrap_err()),
        "floors"
    );
}

#[test]
fn a_member_refusal_is_recorded_without_text_and_audited() {
    let g = members_gateway();
    let _ = g.admin_close_room(&alice(), "r", true);
    let refusals = g.admin_refusals(&member("ada"), 5).unwrap();
    assert!(
        refusals.iter().any(|r| r.rule == "admin-scope"),
        "{refusals:?}"
    );
    let log = g.admin_audit(&member("ada"), 5).unwrap();
    assert!(
        log.iter()
            .any(|a| a.action == "refused" && a.detail.contains("alice")),
        "{log:?}"
    );
}

#[test]
fn the_admin_agent_still_does_its_acts_on_a_member_gateway() {
    let g = members_gateway();
    g.admin_close_room(&member("ada"), "r", true).unwrap();
    assert!(is_refused(g.post(&alice(), "r", "closed")));
    g.admin_close_room(&member("ada"), "r", false).unwrap();
    assert_eq!(
        rule_of(g.admin_set_cap(&member("ada"), "internal").unwrap_err()),
        "floors"
    );
}

#[test]
fn housekeeping_at_session_start_is_not_an_admin_act_for_a_member() {
    let g = members_gateway();
    assert!(g.prune_members(&alice(), 86_400).is_ok());
}

#[test]
fn a_member_still_reads_any_hook_record_as_before() {
    // `klatch hooks --of <member>` is a content-free diagnostic members have always had; narrowing it changes who may
    // do what, so it is left as it was and ticketed for the human to decide
    let g = members_gateway();
    assert!(g.hook_runs(&alice(), "alice").is_ok());
    assert!(g.hook_runs(&alice(), "bob").is_ok());
}

#[test]
fn a_librarian_may_read_the_records_and_nothing_more_on_a_member_gateway() {
    let g = members_gateway();
    assert!(g.admin_audit(&member("lib"), 5).is_ok());
    assert!(g.admin_refusals(&member("lib"), 5).is_ok());
    assert_eq!(
        rule_of(g.admin_close_room(&member("lib"), "r", true).unwrap_err()),
        "read-only"
    );
}

#[test]
fn a_gateway_not_built_for_members_is_unchanged() {
    let (g, _) = timed();
    g.join(&alice(), "r").unwrap();
    g.admin_close_room(&admin(), "r", true).unwrap();
    assert!(g.admin_audit(&admin(), 5).is_ok());
}
