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

#[test]
fn a_claim_lives_on_while_its_owner_stays_active() {
    let (g, t) = timed();
    g.join(&alice(), "r").unwrap();
    g.claim(&alice(), "r", &ClaimSpec::new("p", 600, ""))
        .unwrap();
    for _ in 0..3 {
        t.set(t.get() + 500);
        g.digest(&alice()).unwrap();
    }
    // 1500 s after the claim, longer than its lease, the owner kept working (#494).
    assert!(held_by_anyone(&g, "p"));
    assert!(g.claim(&bob(), "r", &ClaimSpec::new("p", 100, "")).is_err());
}

#[test]
fn a_claim_of_a_silent_owner_still_lapses_at_its_lease() {
    let (g, t) = timed();
    g.claim(&alice(), "r", &ClaimSpec::new("p", 600, ""))
        .unwrap();
    t.set(t.get() + 601);
    assert!(!held_by_anyone(&g, "p"));
}

#[test]
fn activity_never_shortens_a_longer_lease() {
    let (g, t) = timed();
    g.join(&alice(), "r").unwrap();
    g.claim(&alice(), "r", &ClaimSpec::new("long", 7_200, ""))
        .unwrap();
    t.set(t.get() + 100);
    g.digest(&alice()).unwrap();
    t.set(t.get() + 7_000);
    assert!(held_by_anyone(&g, "long"));
}
