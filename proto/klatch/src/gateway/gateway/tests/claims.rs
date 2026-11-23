use super::*;

#[test]
fn two_scopes_can_claim_the_same_path_and_one_scope_cannot_claim_it_twice() {
    let g = open();
    let (a, b, c) = (alice(), bob(), Principal::unattributed("carol"));
    g.set_scope(&a, "git:a").unwrap();
    g.set_scope(&b, "git:b").unwrap();
    g.set_scope(&c, "git:a").unwrap();
    g.claim(&a, "r", &ClaimSpec::new("src/lib.rs", 100, ""))
        .unwrap();
    g.claim(&b, "r", &ClaimSpec::new("src/lib.rs", 100, ""))
        .unwrap();
    assert!(matches!(
        g.claim(&c, "r", &ClaimSpec::new("src/lib.rs", 100, "")),
        Err(Error::Conflict { .. })
    ));
    assert_eq!(g.claims(&a, "r").unwrap().len(), 2);
}

#[test]
fn members_with_no_scope_still_collide_as_before() {
    let g = open();
    g.claim(&alice(), "r", &ClaimSpec::new("p", 100, ""))
        .unwrap();
    assert!(g.claim(&bob(), "r", &ClaimSpec::new("p", 100, "")).is_err());
}

fn held_by_anyone(g: &Gateway<SpyStore>, path: &str) -> bool {
    g.claims(&bob(), "r")
        .unwrap()
        .iter()
        .any(|c| c.path == path)
}

fn claimed(g: &Gateway<SpyStore>, path: &str, ttl: i64) {
    g.join(&alice(), "r").unwrap();
    g.claim(&alice(), "r", &ClaimSpec::new(path, ttl, ""))
        .unwrap();
}

#[test]
fn a_claim_is_renewed_back_to_its_own_ttl_not_to_a_floor() {
    let (g, t) = timed();
    claimed(&g, "short", 60);
    t.set(t.get() + 50);
    g.digest(&alice()).unwrap();
    // 100 s after the claim: past its first lease, inside 60 s of the activity at +50.
    t.set(t.get() + 50);
    assert!(held_by_anyone(&g, "short"));
    // 61 s after the last activity: a 60 s claim is not stretched to a 600 s floor (#494).
    t.set(t.get() + 61);
    assert!(!held_by_anyone(&g, "short"));
}

#[test]
fn a_long_claim_is_renewed_to_its_own_long_ttl() {
    let (g, t) = timed();
    claimed(&g, "long", 3_600);
    t.set(t.get() + 3_000);
    g.digest(&alice()).unwrap();
    t.set(t.get() + 3_500);
    assert!(held_by_anyone(&g, "long"));
    t.set(t.get() + 200);
    assert!(!held_by_anyone(&g, "long"));
}

#[test]
fn a_lapsed_claim_is_not_revived_by_later_activity() {
    let (g, t) = timed();
    claimed(&g, "p", 60);
    t.set(t.get() + 61);
    g.digest(&alice()).unwrap();
    assert!(!held_by_anyone(&g, "p"));
}

#[test]
fn an_ask_lease_is_not_renewed_by_activity() {
    let (g, t) = timed();
    claimed(&g, "ask:7", 100);
    t.set(t.get() + 90);
    g.digest(&alice()).unwrap();
    t.set(t.get() + 20);
    assert!(!held_by_anyone(&g, "ask:7"));
}

#[test]
fn renewal_never_passes_the_maximum_ttl() {
    use crate::domain::CLAIM_MAX_SECS;
    let (g, t) = timed();
    claimed(&g, "max", CLAIM_MAX_SECS);
    t.set(t.get() + 1_000);
    g.digest(&alice()).unwrap();
    // The lease from the last activity is the claim's own ttl, never a floor above the maximum.
    t.set(t.get() + CLAIM_MAX_SECS + 1);
    assert!(!held_by_anyone(&g, "max"));
}

#[test]
fn another_members_activity_does_not_renew_a_claim() {
    let (g, t) = timed();
    claimed(&g, "p", 60);
    g.join(&bob(), "r").unwrap();
    t.set(t.get() + 50);
    g.digest(&bob()).unwrap();
    t.set(t.get() + 20);
    assert!(!held_by_anyone(&g, "p"));
}

#[test]
fn claims_through_the_gateway_leave_event_rows_and_a_release_carries_its_hold_time() {
    let (g, t) = timed();
    g.join(&alice(), "lobby").unwrap();
    g.join(&bob(), "lobby").unwrap();
    t.set(1_000);
    g.claim(&alice(), "lobby", &ClaimSpec::new("src/a.rs", 3_600, "x"))
        .unwrap();
    t.set(1_010);
    g.claim(&bob(), "lobby", &ClaimSpec::new("src/a.rs", 3_600, "x"))
        .unwrap_err();
    t.set(1_090);
    g.release(&alice(), "lobby", "src/a.rs").unwrap();
    t.set(1_100);
    g.claim(&bob(), "lobby", &ClaimSpec::new("src/a.rs", 3_600, "x"))
        .unwrap();
    let h = g.store_for_tests().claim_history_for_test();
    assert_eq!(h.granted, vec![("file".to_string(), 2)]);
    assert_eq!(h.refused, vec![("same-path".to_string(), 1)]);
    assert_eq!(h.released_by, vec![("holder".to_string(), 1)]);
    assert_eq!((h.handled, h.handled_wait_median), (1, Some(90)));
    assert_eq!((h.holds, h.hold_median), (1, Some(90)));
}
