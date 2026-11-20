//! The `join` and `asks` wire methods (#901): a member makes or joins a room and lists a room's open asks over the
//! wire, through the same gateway calls as `klatch join` and `klatch asks`, so an MCP-only facilitator can route.

use super::*;

fn call(
    g: &Gateway<SqliteStore>,
    who: &Principal,
    method: &str,
    params: Value,
) -> (bool, Option<Value>, Option<(String, String)>) {
    response(&run(g, who, &[req("x", method, params)]), "x")
}

#[test]
fn join_makes_the_room_and_the_caller_is_a_member() {
    let g = gateway();
    let (ok, result, err) = call(&g, &alice(), "join", json!({"room": "newroom"}));
    assert!(ok, "{err:?}");
    assert_eq!(result.unwrap(), json!({}));
    let (ok, result, _) = call(&g, &alice(), "who", json!({"room": "newroom"}));
    assert!(ok);
    assert_eq!(result.unwrap()["members"][0]["name"], "alice");
}

#[test]
fn join_without_a_room_is_invalid_params_and_a_bad_name_is_invalid() {
    let g = gateway();
    let (ok, _, err) = call(&g, &alice(), "join", json!({}));
    assert!(!ok);
    assert_eq!(err.unwrap().0, "invalid_params");
    let (ok, _, err) = call(&g, &alice(), "join", json!({"room": "a b/../c"}));
    assert!(!ok);
    assert_eq!(err.unwrap().0, "invalid");
}

#[test]
fn a_member_param_does_not_join_someone_else() {
    let g = gateway();
    call(&g, &alice(), "join", json!({"room": "r", "member": "bob"}));
    let (_, result, _) = call(&g, &alice(), "who", json!({"room": "r"}));
    assert_eq!(result.unwrap()["members"].as_array().unwrap().len(), 1);
}

#[test]
fn asks_lists_the_open_asks_with_their_state() {
    let g = gateway();
    g.join(&alice(), "r").unwrap();
    g.join(&bob(), "r").unwrap();
    g.post(&alice(), "r", "seeking: a review of the parser")
        .unwrap();
    let (ok, result, err) = call(&g, &bob(), "asks", json!({"room": "r"}));
    assert!(ok, "{err:?}");
    let asks = result.unwrap()["asks"].as_array().unwrap().clone();
    assert_eq!(asks.len(), 1, "{asks:?}");
    assert_eq!(asks[0]["sender"], "alice");
    assert_eq!(asks[0]["claimed"], false);
    assert_eq!(asks[0]["waiting_on_asker"], false);
    assert!(asks[0]["seq"].is_i64());
    assert!(asks[0]["text"].as_str().unwrap().contains("parser"));
}

#[test]
fn asks_needs_a_room_and_a_room_with_no_asks_is_an_empty_list() {
    let g = gateway();
    g.join(&alice(), "r").unwrap();
    let (ok, _, err) = call(&g, &alice(), "asks", json!({}));
    assert!(!ok);
    assert_eq!(err.unwrap().0, "invalid_params");
    let (ok, result, _) = call(&g, &alice(), "asks", json!({"room": "r"}));
    assert!(ok);
    assert_eq!(result.unwrap(), json!({"asks": []}));
}

#[test]
fn asks_of_a_room_above_the_callers_clearance_is_refused() {
    let g = gateway();
    let human = Principal::unattributed("admin");
    g.join(&alice(), "vault").unwrap();
    g.admin_set_room_label(&human, "vault", "private").unwrap();
    let (ok, _, err) = call(&g, &bob(), "asks", json!({"room": "vault"}));
    assert!(!ok);
    assert_eq!(err.unwrap().0, "refused");
}

#[test]
fn the_facilitator_can_join_and_list_asks_but_a_librarian_cannot_join() {
    let g = gateway();
    let human = Principal::unattributed("admin");
    g.join(&alice(), "lobby").unwrap();
    g.post(&alice(), "lobby", "seeking: help with the login flow")
        .unwrap();
    g.admin_set_facilitator(&human, "fac", true).unwrap();
    let fac = Principal::unattributed("fac");
    let (ok, _, err) = call(&g, &fac, "join", json!({"room": "lobby"}));
    assert!(ok, "{err:?}");
    let (ok, result, err) = call(&g, &fac, "asks", json!({"room": "lobby"}));
    assert!(ok, "{err:?}");
    assert_eq!(result.unwrap()["asks"].as_array().unwrap().len(), 1);
    g.admin_set_librarian(&human, "lib", true).unwrap();
    let lib = Principal::unattributed("lib");
    let (ok, _, err) = call(&g, &lib, "join", json!({"room": "other"}));
    assert!(!ok, "a librarian made a room");
    assert_eq!(err.unwrap().0, "refused");
}
