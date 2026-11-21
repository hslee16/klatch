//! #893 (decision on #887): the creator of a room is its initial grantor of room-scoped clearance, within strict caps:
//! never above its own global clearance, the room's label or the agent cap, never the top level, never to itself,
//! never outside its room. The human removes or replaces the grantor and revokes any grant. Every test below is an
//! attempt to get more than the grantor holds; the decision reads only the policy table and the roles, never text.

use super::*;
use crate::gateway::ForbidFloors;

fn carol() -> Principal {
    Principal::unattributed("carol")
}

/// `vault` is private and was made by alice (so alice is its grantor), cleared to private. bob and carol are at the
/// lowest clearance. All three share `lobby`.
fn vault() -> Gateway<SpyStore> {
    let g = open();
    for p in [alice(), bob(), carol()] {
        g.join(&p, "lobby").unwrap();
    }
    g.join(&alice(), "vault").unwrap();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g
}

fn grant_rows(g: &Gateway<SpyStore>) -> Vec<crate::domain::AuditRecord> {
    g.store
        .audit_log(100)
        .unwrap()
        .into_iter()
        .filter(|a| a.action == "grant-clearance")
        .collect()
}

fn refused_with<T: std::fmt::Debug>(r: Result<T>, rule: &str) {
    match r {
        Err(Error::Refused { rule: got, .. }) => assert_eq!(got, rule),
        other => panic!("expected refusal {rule}, got {other:?}"),
    }
}

#[test]
fn the_creator_of_a_room_is_its_initial_grantor() {
    let g = vault();
    assert_eq!(g.room_grantor("vault").unwrap().as_deref(), Some("alice"));
    // a room only joined, not made, has the grantor of its creator
    assert_eq!(g.room_grantor("lobby").unwrap().as_deref(), Some("alice"));
    assert_eq!(g.room_grantor("nowhere").unwrap(), None);
}

#[test]
fn creating_a_room_by_posting_also_makes_the_poster_its_grantor() {
    let g = open();
    g.post(&bob(), "fresh", "hello").unwrap();
    assert_eq!(g.room_grantor("fresh").unwrap().as_deref(), Some("bob"));
}

#[test]
fn the_grantor_grants_within_its_clearance_and_the_room_label_and_the_grantee_reads_only_that_room()
{
    let g = vault();
    g.admin_set_room_label(&admin(), "other", "private")
        .unwrap();
    refused_with(g.join(&bob(), "vault"), "clearance");
    g.grant_room_clearance(&alice(), "vault", "bob", "private")
        .unwrap();
    g.join(&bob(), "vault").unwrap();
    g.post(&bob(), "vault", "in").unwrap();
    // the global clearance did not move
    assert_eq!(g.clearance_of("bob").unwrap(), "public");
    // and another private room is still out of reach
    g.join(&alice(), "other").unwrap();
    refused_with(g.join(&bob(), "other"), "clearance");
}

#[test]
fn a_grant_makes_an_invite_to_that_room_possible() {
    let g = vault();
    refused_with(g.invite(&alice(), "bob", "vault", "lobby"), "clearance");
    g.grant_room_clearance(&alice(), "vault", "bob", "private")
        .unwrap();
    g.invite(&alice(), "bob", "vault", "lobby").unwrap();
}

#[test]
fn a_grant_above_the_room_label_is_refused_with_the_rule() {
    let g = vault();
    g.admin_set_room_label(&admin(), "vault", "internal")
        .unwrap();
    refused_with(
        g.grant_room_clearance(&alice(), "vault", "bob", "private"),
        "room-label",
    );
    g.grant_room_clearance(&alice(), "vault", "bob", "internal")
        .unwrap();
}

#[test]
fn a_grant_above_the_grantors_global_clearance_is_refused_even_inside_a_high_room() {
    // the room is private, the grantor holds a room grant to private only (global public): room grants are not
    // transitive, so as grantor it can pass on only its global clearance
    let g = vault();
    g.admin_set_room_grantor(&admin(), "vault", "bob", true)
        .unwrap();
    g.admin_set_room_clearance(&admin(), "vault", "bob", Some("private"))
        .unwrap();
    g.join(&bob(), "vault").unwrap();
    refused_with(
        g.grant_room_clearance(&bob(), "vault", "carol", "private"),
        "own-clearance",
    );
    refused_with(
        g.grant_room_clearance(&bob(), "vault", "carol", "internal"),
        "own-clearance",
    );
    g.grant_room_clearance(&bob(), "vault", "carol", "public")
        .unwrap();
}

#[test]
fn a_grantor_may_not_grant_above_the_agent_cap_or_the_top_level() {
    let g = vault();
    // alice is private; the top level is never grantable, whoever holds what
    refused_with(
        g.grant_room_clearance(&alice(), "vault", "bob", "secret"),
        "top-level",
    );
    // a store hand-edited to a cap at the top still cannot make the top level grantable
    g.store.policy_set("cap", Some("secret"), 1).unwrap();
    refused_with(
        g.grant_room_clearance(&alice(), "vault", "bob", "secret"),
        "top-level",
    );
    // a lowered cap: a label above it is refused as the cap
    g.store.policy_set("cap", Some("internal"), 2).unwrap();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    g.store
        .policy_set("clearance:alice", Some("private"), 3)
        .unwrap();
    refused_with(
        g.grant_room_clearance(&alice(), "vault", "bob", "private"),
        "agent-cap",
    );
    g.grant_room_clearance(&alice(), "vault", "bob", "internal")
        .unwrap();
}

#[test]
fn a_grantor_cannot_grant_itself() {
    let g = vault();
    for label in ["public", "internal", "private"] {
        refused_with(
            g.grant_room_clearance(&alice(), "vault", "alice", label),
            "self-grant",
        );
    }
    assert!(grant_rows(&g).is_empty());
}

#[test]
fn a_grantor_cannot_grant_in_a_room_it_does_not_grant_for() {
    let g = vault();
    g.join(&bob(), "bobs-room").unwrap();
    g.admin_set_room_label(&admin(), "bobs-room", "private")
        .unwrap();
    // alice is a member of neither the room nor its grantor
    refused_with(
        g.grant_room_clearance(&alice(), "bobs-room", "carol", "private"),
        "grantor",
    );
    // a member that is not the grantor of a room it is in
    g.join(&bob(), "vault").unwrap_err();
    g.grant_room_clearance(&alice(), "vault", "bob", "private")
        .unwrap();
    g.join(&bob(), "vault").unwrap();
    refused_with(
        g.grant_room_clearance(&bob(), "vault", "carol", "private"),
        "grantor",
    );
    assert!(
        g.store
            .policy_get("room-clear:bobs-room:carol")
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_grant_in_a_room_never_widens_another_room() {
    let g = vault();
    g.join(&alice(), "second").unwrap();
    g.admin_set_room_label(&admin(), "second", "private")
        .unwrap();
    g.grant_room_clearance(&alice(), "vault", "bob", "private")
        .unwrap();
    refused_with(g.join(&bob(), "second"), "clearance");
}

#[test]
fn the_human_removes_the_grantor_and_a_removed_grantor_can_grant_nothing() {
    let g = vault();
    g.admin_set_room_grantor(&admin(), "vault", "alice", false)
        .unwrap();
    assert_eq!(g.room_grantor("vault").unwrap(), None);
    refused_with(
        g.grant_room_clearance(&alice(), "vault", "bob", "private"),
        "grantor",
    );
    assert!(grant_rows(&g).is_empty());
}

#[test]
fn removing_someone_who_is_not_the_grantor_changes_nothing() {
    let g = vault();
    g.admin_set_room_grantor(&admin(), "vault", "bob", false)
        .unwrap();
    assert_eq!(g.room_grantor("vault").unwrap().as_deref(), Some("alice"));
}

#[test]
fn the_human_replaces_the_grantor() {
    let g = vault();
    g.admin_set_clearance(&admin(), "carol", "private").unwrap();
    g.admin_set_room_grantor(&admin(), "vault", "carol", true)
        .unwrap();
    assert_eq!(g.room_grantor("vault").unwrap().as_deref(), Some("carol"));
    refused_with(
        g.grant_room_clearance(&alice(), "vault", "bob", "private"),
        "grantor",
    );
    g.grant_room_clearance(&carol(), "vault", "bob", "private")
        .unwrap();
}

#[test]
fn a_grantor_that_became_a_librarian_or_reviewer_can_not_grant() {
    for make in [
        |g: &Gateway<SpyStore>| g.admin_set_librarian(&admin(), "alice", true),
        |g: &Gateway<SpyStore>| g.admin_set_reviewer(&admin(), "alice", true),
    ] {
        let g = vault();
        make(&g).unwrap();
        assert!(is_refused(g.grant_room_clearance(
            &alice(),
            "vault",
            "bob",
            "private"
        )));
        assert!(grant_rows(&g).is_empty());
    }
}

#[test]
fn a_grantor_whose_own_clearance_dropped_can_no_longer_grant_what_it_lost() {
    let g = vault();
    g.admin_set_clearance(&admin(), "alice", "public").unwrap();
    assert!(is_refused(g.grant_room_clearance(
        &alice(),
        "vault",
        "bob",
        "private"
    )));
    assert!(grant_rows(&g).is_empty());
}

#[test]
fn an_unknown_label_and_a_bad_name_are_not_grants() {
    let g = vault();
    assert!(is_invalid(g.grant_room_clearance(
        &alice(),
        "vault",
        "bob",
        "nonsense"
    )));
    assert!(is_invalid(g.grant_room_clearance(
        &alice(),
        "vault",
        "bad name",
        "private"
    )));
    assert!(is_invalid(g.grant_room_clearance(
        &alice(),
        "bad room",
        "bob",
        "private"
    )));
    assert!(grant_rows(&g).is_empty());
}

#[test]
fn every_grant_is_audited_with_grantor_grantee_room_and_label() {
    let g = vault();
    g.grant_room_clearance(&alice(), "vault", "bob", "internal")
        .unwrap();
    let rows = grant_rows(&g);
    assert_eq!(rows.len(), 1);
    let text = format!("{} {}", rows[0].target, rows[0].detail);
    for word in ["alice", "bob", "vault", "internal"] {
        assert!(text.contains(word), "{word} missing from {text}");
    }
}

#[test]
fn a_refused_grant_changes_nothing_writes_no_grant_row_and_keeps_a_refusal() {
    let g = vault();
    refused_with(
        g.grant_room_clearance(&alice(), "vault", "alice", "private"),
        "self-grant",
    );
    assert!(grant_rows(&g).is_empty());
    assert!(
        g.store
            .policy_get("room-clear:vault:alice")
            .unwrap()
            .is_none()
    );
    let seen = g.my_refusals(&alice(), 10).unwrap();
    assert!(
        seen.iter()
            .any(|r| r.rule == "self-grant" && r.room == "vault"),
        "{seen:?}"
    );
}

#[test]
fn the_human_revokes_a_grant_and_the_grantee_is_refused_again() {
    let g = vault();
    g.grant_room_clearance(&alice(), "vault", "bob", "private")
        .unwrap();
    g.join(&bob(), "vault").unwrap();
    g.admin_set_room_clearance(&admin(), "vault", "bob", None)
        .unwrap();
    refused_with(g.read(&bob(), "vault", false), "clearance");
    assert!(
        g.admin_audit(&admin(), 100)
            .unwrap()
            .iter()
            .any(|a| a.action == "room-clear")
    );
}

#[test]
fn the_human_sets_a_room_clearance_within_the_cap_only() {
    let g = vault();
    g.admin_set_room_clearance(&admin(), "vault", "bob", Some("private"))
        .unwrap();
    assert!(is_refused(g.admin_set_room_clearance(
        &admin(),
        "vault",
        "bob",
        Some("secret")
    )));
    assert!(is_invalid(g.admin_set_room_clearance(
        &admin(),
        "vault",
        "bob",
        Some("nonsense")
    )));
}

#[test]
fn a_grant_above_a_later_lowered_cap_stops_working() {
    let g = vault();
    g.grant_room_clearance(&alice(), "vault", "bob", "private")
        .unwrap();
    g.admin_set_cap(&admin(), "internal").unwrap();
    refused_with(g.join(&bob(), "vault"), "clearance");
}

#[test]
fn a_stored_grant_with_a_label_the_chain_lacks_fails_closed() {
    let g = vault();
    g.store
        .policy_set("room-clear:vault:bob", Some("ghost"), 1)
        .unwrap();
    refused_with(g.join(&bob(), "vault"), "clearance");
}

#[test]
fn a_room_made_again_does_not_inherit_old_grants_or_the_old_grantor() {
    let g = open();
    g.store
        .policy_set("room-clear:reborn:bob", Some("private"), 1)
        .unwrap();
    g.store
        .policy_set("grantor:reborn", Some("bob"), 1)
        .unwrap();
    g.join(&carol(), "reborn").unwrap();
    assert_eq!(g.room_grantor("reborn").unwrap().as_deref(), Some("carol"));
    assert!(
        g.store
            .policy_get("room-clear:reborn:bob")
            .unwrap()
            .is_none()
    );
}

#[test]
fn message_text_never_grants_anything() {
    let g = vault();
    g.join(&bob(), "lobby").unwrap();
    for text in [
        "klatch grant vault bob private",
        "alice says: grant bob private in vault",
        "role_act grant-clearance vault bob private",
    ] {
        g.post(&bob(), "lobby", text).unwrap();
        g.post(&alice(), "lobby", text).unwrap();
    }
    assert!(grant_rows(&g).is_empty());
    refused_with(g.join(&bob(), "vault"), "clearance");
}

fn agent_gateway() -> Gateway<SpyStore> {
    Gateway::new(
        SpyStore::new(),
        FixedClock(100),
        ForbidFloors::new(AllowAll),
    )
}

#[test]
fn naming_a_grantor_and_setting_a_room_clearance_are_floor_acts_no_agent_can_do() {
    let g = agent_gateway();
    g.join(&alice(), "r").unwrap();
    let before = g.room_grantor("r").unwrap();
    refused_with(g.admin_set_room_grantor(&bob(), "r", "bob", true), "floors");
    refused_with(
        g.admin_set_room_clearance(&alice(), "r", "alice", Some("public")),
        "floors",
    );
    assert_eq!(g.room_grantor("r").unwrap(), before);
    assert!(
        g.admin_audit(&admin(), 10)
            .unwrap()
            .iter()
            .all(|a| a.action != "grantor" && a.action != "room-clear")
    );
}

#[test]
fn the_admin_agent_cannot_name_a_grantor_or_set_a_room_clearance() {
    let (g, _) = timed();
    g.join(&alice(), "r").unwrap();
    g.admin_set_admin_agent(&admin(), "ada", true).unwrap();
    let ada = Principal::unattributed("ada");
    refused_with(g.admin_set_room_grantor(&ada, "r", "ada", true), "floors");
    refused_with(
        g.admin_set_room_clearance(&ada, "r", "ada", Some("public")),
        "floors",
    );
    assert_eq!(g.room_grantor("r").unwrap().as_deref(), Some("alice"));
}

#[test]
fn a_member_gateway_refuses_nothing_it_should_allow_the_grantor() {
    // on the gateway agents reach (floors forbidden, members' admin acts need a role) the grantor still grants,
    // because granting is a member operation checked against the grantor role, not an admin act
    let g = Gateway::new(
        SpyStore::new(),
        FixedClock(100),
        ForbidFloors::new(AllowAll),
    )
    .for_members();
    g.join(&alice(), "r").unwrap();
    g.grant_room_clearance(&alice(), "r", "bob", "public")
        .unwrap();
    assert_eq!(grant_rows(&g).len(), 1);
}

#[test]
fn ending_a_run_drops_the_grantor_and_the_grants_of_its_rooms_and_only_those() {
    let g = open();
    g.admin_run_create(&admin(), "a").unwrap();
    g.join(&alice(), "run.a.main").unwrap();
    g.join(&alice(), "run.ab").unwrap_err();
    g.join(&alice(), "keep").unwrap();
    g.store
        .policy_set("room-clear:run.a.main:bob", Some("internal"), 1)
        .unwrap();
    g.grant_room_clearance(&alice(), "keep", "bob", "public")
        .unwrap();
    g.admin_run_end(&admin(), "a", None).unwrap();
    assert_eq!(g.room_grantor("run.a.main").unwrap(), None);
    assert!(
        g.store
            .policy_get("room-clear:run.a.main:bob")
            .unwrap()
            .is_none()
    );
    assert_eq!(g.room_grantor("keep").unwrap().as_deref(), Some("alice"));
    assert!(g.store.policy_get("room-clear:keep:bob").unwrap().is_some());
}

/// #897: the grantor check and the grant write are one transaction. Another connection (the human's `grantor --off`)
/// tries to remove the grantor in the window just before the grant row is written; it must not get in, so a grant
/// row never outlives the grantor that was checked for it.
#[test]
fn the_grantor_check_and_the_grant_write_are_one_transaction() {
    use crate::store::{SqliteStore, insecure_test_key};
    use std::time::Duration;

    let dir = crate::store::test_dir::temp_db("grant-atomic");
    let key = insecure_test_key();
    let open_at =
        |ms| SqliteStore::open_with_timeout(&dir, &key, Duration::from_millis(ms)).unwrap();
    let g = Gateway::new(SpyStore::over(open_at(5000)), FixedClock(100), AllowAll);
    for p in [alice(), bob()] {
        g.join(&p, "lobby").unwrap();
    }
    g.join(&alice(), "vault").unwrap();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();

    let rival = open_at(100);
    let rival_result = Rc::new(std::cell::RefCell::new(None));
    let seen = rival_result.clone();
    *g.store.before_policy_set.borrow_mut() = Some(Box::new(move |key| {
        if key.starts_with("room-clear:") {
            *seen.borrow_mut() = Some(rival.policy_set("grantor:vault", None, 1));
        }
    }));

    g.grant_room_clearance(&alice(), "vault", "bob", "private")
        .unwrap();

    let rival_outcome = rival_result.borrow_mut().take();
    assert!(
        matches!(rival_outcome, Some(Err(_))),
        "the grantor was removed between the check and the write: {rival_outcome:?}"
    );
    assert_eq!(g.room_grantor("vault").unwrap().as_deref(), Some("alice"));
    assert_eq!(grant_rows(&g).len(), 1);
}

// ---- #896: a room grant never hands out a label key ----

#[test]
fn a_grant_into_a_sealed_room_is_refused_when_the_grantee_lacks_the_global_clearance() {
    let g = vault();
    g.admin_seal_label_keep_plain(&admin(), "private").unwrap();
    let r = g.grant_room_clearance(&alice(), "vault", "bob", "private");
    match r {
        Err(Error::Refused { rule, reason }) => {
            assert_eq!(rule, "sealed-key");
            assert!(
                reason.contains("klatch admin clear bob private"),
                "{reason}"
            );
        }
        other => panic!("expected refusal sealed-key, got {other:?}"),
    }
    assert!(grant_rows(&g).is_empty());
}

#[test]
fn a_grant_into_a_sealed_room_is_allowed_when_the_grantee_already_holds_the_clearance() {
    let g = vault();
    g.admin_set_clearance(&admin(), "bob", "private").unwrap();
    g.admin_seal_label_keep_plain(&admin(), "private").unwrap();
    g.grant_room_clearance(&alice(), "vault", "bob", "private")
        .unwrap();
}

#[test]
fn an_unsealed_room_still_takes_a_grant() {
    let g = vault();
    g.admin_seal_label_keep_plain(&admin(), "secret").unwrap();
    g.grant_room_clearance(&alice(), "vault", "bob", "private")
        .unwrap();
}
