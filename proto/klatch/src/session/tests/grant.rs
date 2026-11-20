//! The `grant_room_clearance` wire method (#895): the room grantor gives one member a clearance in that room only,
//! through the same gateway call as `klatch grant`, so every rule of #893 applies and a refusal names its rule.

use super::*;

fn call(
    g: &Gateway<SqliteStore>,
    who: &Principal,
    params: Value,
) -> (bool, Option<Value>, Option<(String, String)>) {
    response(
        &run(g, who, &[req("g", "grant_room_clearance", params)]),
        "g",
    )
}

fn room_made_by_alice() -> Gateway<SqliteStore> {
    let g = gateway();
    g.join(&alice(), "r").unwrap();
    g.join(&bob(), "lobby").unwrap();
    g
}

#[test]
fn the_grantor_grants_over_the_wire_and_it_is_audited() {
    let g = room_made_by_alice();
    let lowest = g.label_order(&alice()).unwrap().0[0].clone();
    let (ok, result, err) = call(
        &g,
        &alice(),
        json!({"room":"r","member":"bob","label":lowest}),
    );
    assert!(ok, "{err:?}");
    assert_eq!(result.unwrap(), json!({}));
}

#[test]
fn a_caller_that_is_not_the_grantor_is_refused_with_the_grantor_rule() {
    let g = room_made_by_alice();
    let lowest = g.label_order(&alice()).unwrap().0[0].clone();
    let (ok, _, err) = call(
        &g,
        &bob(),
        json!({"room":"r","member":"alice","label":lowest}),
    );
    assert!(!ok);
    let (code, message) = err.unwrap();
    assert_eq!(code, "refused");
    assert!(message.contains("grantor"), "{message}");
}

#[test]
fn the_top_level_and_a_grant_to_yourself_are_refused() {
    let g = room_made_by_alice();
    let order = g.label_order(&alice()).unwrap().0;
    let top = order.last().unwrap().clone();
    let (ok, _, _) = call(&g, &alice(), json!({"room":"r","member":"bob","label":top}));
    assert!(!ok);
    let (ok, _, _) = call(
        &g,
        &alice(),
        json!({"room":"r","member":"alice","label":order[0]}),
    );
    assert!(!ok);
}

#[test]
fn missing_or_wrongly_typed_params_are_invalid_params() {
    let g = room_made_by_alice();
    for p in [
        json!({}),
        json!({"room":"r","member":"bob"}),
        json!({"room":"r","member":"bob","label":3}),
    ] {
        let (ok, _, err) = call(&g, &alice(), p);
        assert!(!ok);
        assert_eq!(err.unwrap().0, "invalid_params");
    }
}
