//! The `stuck` wire method (#271): the stuck-case events of a room, ids and numbers only.

use super::*;

fn call(g: &Gateway<SqliteStore>, who: &Principal, params: Value) -> (bool, Option<Value>, Option<(String, String)>) {
    response(&run(g, who, &[req("s", "stuck", params)]), "s")
}

#[test]
fn a_cycle_through_asks_is_listed_with_members_and_ask_numbers_and_no_text() {
    let g = gateway();
    let post = |who: &Principal, body: &str| {
        let f = run(&g, who, &[req("p", "post", json!({"room":"r","body":body}))]);
        response(&f, "p").1.unwrap()["seq"].as_i64().unwrap()
    };
    let a = post(&alice(), "seeking: SECRET-WORDING one");
    let b = post(&bob(), "seeking: SECRET-WORDING two");
    let claim = |who: &Principal, n: i64| {
        let f = run(&g, who, &[req("c", "claim", json!({"room":"r","path":format!("ask:{n}")}))]);
        assert!(response(&f, "c").0);
    };
    claim(&bob(), a);
    claim(&alice(), b);
    let (ok, result, _) = call(&g, &bob(), json!({"room":"r"}));
    assert!(ok);
    let r = result.unwrap();
    let ev = r["events"].as_array().unwrap();
    assert_eq!(ev.len(), 1, "{r}");
    assert_eq!(ev[0]["kind"], "ask_cycle");
    assert_eq!(ev[0]["members"], json!(["alice", "bob"]));
    assert_eq!(ev[0]["asks"], json!([a, b]));
    assert_eq!(ev[0]["key"], format!("ask_cycle:alice,bob:{a},{b}"));
    assert!(!r.to_string().contains("SECRET"), "{r}");
}

#[test]
fn a_missing_room_is_invalid_params_and_a_denied_reader_is_refused() {
    let g = gateway();
    let (ok, _, err) = call(&g, &alice(), json!({}));
    assert!(!ok);
    assert_eq!(err.unwrap().0, "invalid_params");
    let denied = Principal::unattributed("");
    assert!(!call(&g, &denied, json!({"room":"r"})).0);
}
