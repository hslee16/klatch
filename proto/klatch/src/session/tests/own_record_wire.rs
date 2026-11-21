//! The `my_rooms`, `board` and `receipt` wire methods (#848): a wire-only member reads its own rooms, a room's board
//! and the receipts of its own message, the same calls as `klatch who`'s default rooms, `klatch board` and
//! `klatch receipt`.

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
fn my_rooms_lists_only_the_callers_own_rooms() {
    let g = gateway();
    g.join(&alice(), "lobby").unwrap();
    g.join(&alice(), "work").unwrap();
    g.join(&bob(), "other").unwrap();
    let (ok, result, err) = call(&g, &alice(), "my_rooms", json!({}));
    assert!(ok, "{err:?}");
    let mut rooms: Vec<String> = result.unwrap()["rooms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap().to_string())
        .collect();
    rooms.sort();
    assert_eq!(rooms, ["lobby", "work"]);
    // A param naming another member is not read.
    let (_, result, _) = call(&g, &alice(), "my_rooms", json!({"member": "bob"}));
    assert_eq!(result.unwrap()["rooms"].as_array().unwrap().len(), 2);
}

#[test]
fn board_returns_the_rendered_board_as_text() {
    let g = gateway();
    g.join(&alice(), "r").unwrap();
    g.post(&alice(), "r", "offering: review help").unwrap();
    let (ok, result, err) = call(&g, &alice(), "board", json!({"room": "r"}));
    assert!(ok, "{err:?}");
    let v = result.unwrap();
    assert_eq!(v["room"], "r");
    let text = v["board"].as_str().unwrap();
    assert!(text.starts_with("[board r]"), "{text}");
    assert!(text.contains("review help"), "{text}");
}

#[test]
fn board_needs_a_room_param() {
    let g = gateway();
    g.join(&alice(), "r").unwrap();
    let (ok, _, err) = call(&g, &alice(), "board", json!({}));
    assert!(!ok);
    assert_eq!(err.unwrap().0, "invalid_params");
}

#[test]
fn receipt_gives_the_sender_per_recipient_states_and_no_text() {
    let g = gateway();
    g.join(&alice(), "r").unwrap();
    g.join(&bob(), "r").unwrap();
    let posted = g.post(&alice(), "r", "secret words").unwrap();
    let (ok, result, err) = call(
        &g,
        &alice(),
        "receipt",
        json!({"room": "r", "seq": posted.seq}),
    );
    assert!(ok, "{err:?}");
    let v = result.unwrap();
    assert_eq!(v["seq"], posted.seq);
    let rows = v["recipients"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"], "bob");
    assert!(["parked", "seen_after", "not_seen"].contains(&rows[0]["state"].as_str().unwrap()));
    assert!(rows[0]["shown"].is_boolean() && rows[0]["read"].is_boolean());
    assert!(!v.to_string().contains("secret words"));
}

#[test]
fn receipt_is_refused_to_anyone_but_the_sender() {
    let g = gateway();
    g.join(&alice(), "r").unwrap();
    g.join(&bob(), "r").unwrap();
    let posted = g.post(&alice(), "r", "hello").unwrap();
    let (ok, _, err) = call(
        &g,
        &bob(),
        "receipt",
        json!({"room": "r", "seq": posted.seq}),
    );
    assert!(!ok);
    assert_eq!(err.unwrap().0, "refused");
    let (ok, _, err) = call(&g, &alice(), "receipt", json!({"room": "r", "seq": 99}));
    assert!(!ok);
    assert_eq!(err.unwrap().0, "not_found");
    let (ok, _, err) = call(&g, &alice(), "receipt", json!({"room": "r"}));
    assert!(!ok);
    assert_eq!(err.unwrap().0, "invalid_params");
}
