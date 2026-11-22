//! A declared scope on an ask (#1171) on the wire: `post` takes an optional `scope`, and `asks`, `read`, `history`,
//! the stream and `could_answer` carry it. An old client that omits the field posts exactly as before.

use super::*;

fn post(
    g: &Gateway<SqliteStore>,
    params: Value,
) -> (bool, Option<Value>, Option<(String, String)>) {
    response(&run(g, &alice(), &[req("p", "post", params)]), "p")
}

#[test]
fn a_post_with_a_scope_is_an_ask_that_declares_it_and_every_read_path_shows_it() {
    let g = gateway();
    let (ok, result, err) = post(
        &g,
        json!({"room":"r","body":"seeking: a thing","scope":"git:foo"}),
    );
    assert!(ok, "{err:?}");
    let seq = result.unwrap()["seq"].as_i64().unwrap();

    let (_, asks, _) = response(
        &run(&g, &bob(), &[req("a", "asks", json!({"room":"r"}))]),
        "a",
    );
    assert_eq!(asks.unwrap()["asks"][0]["scope"], "git:foo");

    let f = run(
        &g,
        &bob(),
        &[req("s", "subscribe", json!({"room":"r","from_seq":0}))],
    );
    let scopes: Vec<Option<String>> = f
        .iter()
        .filter_map(|x| match x {
            ServerFrame::Message { scope, .. } => Some(scope.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(scopes, vec![Some("git:foo".to_string())]);

    let (_, hist, _) = response(
        &run(&g, &bob(), &[req("h", "history", json!({"room":"r"}))]),
        "h",
    );
    assert_eq!(hist.unwrap()["messages"][0]["scope"], "git:foo");

    let (_, read, _) = response(
        &run(
            &g,
            &bob(),
            &[req("r", "read", json!({"room":"r","peek":true}))],
        ),
        "r",
    );
    assert_eq!(read.unwrap()["messages"][0]["scope"], "git:foo");
    assert_eq!(seq, 1);
}

#[test]
fn an_old_client_that_omits_the_scope_posts_an_ask_with_none_and_nothing_shows_the_key() {
    let g = gateway();
    let (ok, _, err) = post(&g, json!({"room":"r","body":"seeking: a thing"}));
    assert!(ok, "{err:?}");
    let (_, asks, _) = response(
        &run(&g, &bob(), &[req("a", "asks", json!({"room":"r"}))]),
        "a",
    );
    assert!(asks.unwrap()["asks"][0]["scope"].is_null());
    let (_, read, _) = response(
        &run(
            &g,
            &bob(),
            &[req("r", "read", json!({"room":"r","peek":true}))],
        ),
        "r",
    );
    assert!(read.unwrap()["messages"][0].get("scope").is_none());
}

#[test]
fn a_bad_scope_and_a_scope_on_a_plain_post_are_refused_as_invalid_and_post_nothing() {
    let g = gateway();
    for params in [
        json!({"room":"r","body":"seeking: x","scope":"has space"}),
        json!({"room":"r","body":"hello","scope":"git:foo"}),
        json!({"room":"r","body":"seeking: x","scope":"git:foo","kind":"note"}),
    ] {
        let (ok, _, err) = post(&g, params.clone());
        assert!(!ok, "{params}");
        assert_eq!(err.unwrap().0, "invalid", "{params}");
    }
    let (_, read, _) = response(
        &run(
            &g,
            &bob(),
            &[req("r", "read", json!({"room":"r","peek":true}))],
        ),
        "r",
    );
    assert!(read.unwrap()["messages"].as_array().unwrap().is_empty());
}

#[test]
fn could_answer_over_the_wire_names_a_scope_mismatch_without_the_ask_text() {
    let g = gateway();
    run(
        &g,
        &bob(),
        &[
            req("j", "join", json!({"room":"r"})),
            req("sc", "scope", json!({"scope":"git:bar"})),
        ],
    );
    let (_, result, _) = post(
        &g,
        json!({"room":"r","body":"seeking: SECRET-WORDING","scope":"git:foo"}),
    );
    let seq = result.unwrap()["seq"].as_i64().unwrap();
    let (ok, result, _) = response(
        &run(
            &g,
            &bob(),
            &[req(
                "c",
                "could_answer",
                json!({"room":"r","member":"bob","ask":seq}),
            )],
        ),
        "c",
    );
    assert!(ok);
    let result = result.unwrap();
    assert_eq!(result["could"], false);
    assert_eq!(result["blockers"][0]["rule"], "scope");
    assert!(!result.to_string().contains("SECRET-WORDING"));
}
