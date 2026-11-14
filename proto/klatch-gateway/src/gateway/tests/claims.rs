use super::*;

#[test]
fn two_scopes_can_claim_the_same_path_and_one_scope_cannot_claim_it_twice() {
    let g = open();
    let (a, b, c) = (alice(), bob(), Principal::unattributed("carol"));
    g.set_scope(&a, "git:a").unwrap();
    g.set_scope(&b, "git:b").unwrap();
    g.set_scope(&c, "git:a").unwrap();
    g.claim(&a, "r", "src/lib.rs", 100, "").unwrap();
    g.claim(&b, "r", "src/lib.rs", 100, "").unwrap();
    assert!(matches!(
        g.claim(&c, "r", "src/lib.rs", 100, ""),
        Err(Error::Conflict { .. })
    ));
    assert_eq!(g.claims(&a, "r").unwrap().len(), 2);
}

#[test]
fn members_with_no_scope_still_collide_as_before() {
    let g = open();
    g.claim(&alice(), "r", "p", 100, "").unwrap();
    assert!(g.claim(&bob(), "r", "p", 100, "").is_err());
}
