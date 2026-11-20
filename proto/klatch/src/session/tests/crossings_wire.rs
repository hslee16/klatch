//! The `my_crossings` wire method (#961): a member lists its own crossings over the wire, through the same gateway
//! call as `klatch crossings`. Pointers, states and fixed sentences only, never text.

use super::*;
use crate::domain::Verdict;

fn call(
    g: &Gateway<SqliteStore>,
    who: &Principal,
    params: Value,
) -> (bool, Option<Value>, Option<(String, String)>) {
    response(&run(g, who, &[req("x", "my_crossings", params)]), "x")
}

/// `vault` is private with alice cleared to it and one message; `open` is public; rev is a reviewer.
fn world() -> Gateway<SqliteStore> {
    let g = gateway();
    let human = Principal::unattributed("admin");
    g.admin_set_clearance(&human, "alice", "private").unwrap();
    g.admin_set_clearance(&human, "rev", "private").unwrap();
    g.join(&alice(), "vault").unwrap();
    g.admin_set_room_label(&human, "vault", "private").unwrap();
    let posted = g.post(&alice(), "vault", "the plan is blue").unwrap();
    assert_eq!(posted.seq, 2, "the label act plants a canary at 1");
    g.join(&alice(), "open").unwrap();
    g.join(&bob(), "open").unwrap();
    g.admin_set_reviewer(&human, "rev", true).unwrap();
    g
}

#[test]
fn an_empty_list_is_an_empty_array() {
    let g = world();
    let (ok, result, err) = call(&g, &alice(), json!({}));
    assert!(ok, "{err:?}");
    assert_eq!(result.unwrap(), json!({"crossings": []}));
}

#[test]
fn a_waiting_and_a_denied_crossing_carry_state_and_pointers_never_text() {
    let g = world();
    let id = g.request_crossing(&alice(), "vault", 2, "open").unwrap();
    let (_, result, _) = call(&g, &alice(), json!({}));
    let list = result.unwrap()["crossings"].as_array().unwrap().clone();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"], id);
    assert_eq!(list[0]["room"], "vault");
    assert_eq!(list[0]["seq"], 2);
    assert_eq!(list[0]["to_room"], "open");
    assert_eq!(list[0]["state"], "waiting");
    let rev = Principal::unattributed("rev");
    g.review_verdict(
        &rev,
        id,
        &Verdict::Deny("the plan is blue and leaks".into()),
    )
    .unwrap();
    let (_, result, _) = call(&g, &alice(), json!({}));
    let result = result.unwrap();
    assert_eq!(result["crossings"][0]["state"], "denied");
    assert!(!result.to_string().contains("blue"), "{result}");
}

#[test]
fn only_the_callers_own_rows_come_back_and_a_member_param_has_no_effect() {
    let g = world();
    g.request_crossing(&alice(), "vault", 2, "open").unwrap();
    let (ok, result, _) = call(&g, &bob(), json!({"member": "alice", "requester": "alice"}));
    assert!(ok);
    assert_eq!(result.unwrap(), json!({"crossings": []}));
}

#[test]
fn limit_caps_the_list_and_a_wrong_type_is_invalid_params() {
    let g = world();
    g.request_crossing(&alice(), "vault", 2, "open").unwrap();
    g.request_crossing(&alice(), "vault", 2, "open").unwrap();
    let (_, result, _) = call(&g, &alice(), json!({"limit": 1}));
    assert_eq!(result.unwrap()["crossings"].as_array().unwrap().len(), 1);
    let (ok, _, err) = call(&g, &alice(), json!({"limit": "many"}));
    assert!(!ok);
    assert_eq!(err.unwrap().0, "invalid_params");
}
