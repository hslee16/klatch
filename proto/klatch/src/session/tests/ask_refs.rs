//! Refs on an ask (#1318) on the wire: `post` takes an optional `refs` list, and `asks`, `read`, `history`, the
//! stream and `could_answer` carry it. An old client that omits the field posts exactly as before.

use super::*;

fn post(
    g: &Gateway<SqliteStore>,
    params: Value,
) -> (bool, Option<Value>, Option<(String, String)>) {
    response(&run(g, &alice(), &[req("p", "post", params)]), "p")
}

fn read(g: &Gateway<SqliteStore>, method: &str) -> Value {
    let params = if method == "read" {
        json!({"room":"r","peek":true})
    } else {
        json!({"room":"r"})
    };
    response(&run(g, &bob(), &[req("x", method, params)]), "x")
        .1
        .unwrap()
}

#[test]
fn a_post_with_refs_is_an_ask_that_carries_them_and_every_read_path_shows_them() {
    let g = gateway();
    let (ok, _, err) = post(
        &g,
        json!({"room":"r","body":"seeking: a thing","refs":[
            {"kind":"pr","value":"o/r#12","label":"the fix"},
            {"kind":"log","value":"ci.log","snippet":"boom\nbang"},
            {"kind":"figma-frame","value":"login"}
        ]}),
    );
    assert!(ok, "{err:?}");
    let want = json!([
        {"kind":"pr","value":"o/r#12","label":"the fix"},
        {"kind":"log","value":"ci.log","snippet":"boom\nbang"},
        {"kind":"figma-frame","value":"login"}
    ]);
    assert_eq!(read(&g, "asks")["asks"][0]["refs"], want);
    assert_eq!(read(&g, "read")["messages"][0]["refs"], want);
    assert_eq!(read(&g, "history")["messages"][0]["refs"], want);
    let f = run(
        &g,
        &bob(),
        &[req("s", "subscribe", json!({"room":"r","from_seq":0}))],
    );
    let kinds: Vec<Vec<String>> = f
        .iter()
        .filter_map(|x| match x {
            ServerFrame::Message { refs, .. } => {
                Some(refs.iter().map(|r| r.kind.clone()).collect())
            }
            _ => None,
        })
        .collect();
    assert_eq!(kinds, vec![vec!["pr", "log", "figma-frame"]]);
}

#[test]
fn an_old_client_that_omits_refs_posts_an_ask_with_none_and_nothing_shows_the_key() {
    let g = gateway();
    let (ok, _, err) = post(&g, json!({"room":"r","body":"seeking: a thing"}));
    assert!(ok, "{err:?}");
    assert_eq!(read(&g, "asks")["asks"][0]["refs"], json!([]));
    assert!(read(&g, "read")["messages"][0].get("refs").is_none());
    // an empty list is the same as none
    let (ok, _, _) = post(&g, json!({"room":"r","body":"seeking: b","refs":[]}));
    assert!(ok);
}

#[test]
fn bad_refs_and_refs_on_a_plain_post_or_a_handoff_are_refused_as_invalid_and_post_nothing() {
    let g = gateway();
    let many: Vec<Value> = (0..33)
        .map(|i| json!({"kind":"other","value":format!("v{i}")}))
        .collect();
    for params in [
        json!({"room":"r","body":"seeking: x","refs":[{"kind":"Bad Kind","value":"x"}]}),
        json!({"room":"r","body":"seeking: x","refs":[{"kind":"path","value":"/etc/passwd"}]}),
        json!({"room":"r","body":"seeking: x","refs":many}),
        json!({"room":"r","body":"seeking: x","refs":[{"value":"no kind"}]}),
        json!({"room":"r","body":"seeking: x","refs":[{"kind":"pr","value":"#1","nope":1}]}),
        json!({"room":"r","body":"hello","refs":[{"kind":"pr","value":"#1"}]}),
        json!({"room":"r","body":"seeking: x","refs":[{"kind":"pr","value":"#1"}],"kind":"note"}),
        json!({"room":"r","body":"seeking: x","refs":[{"kind":"pr","value":"#1"}],"payload":"p"}),
    ] {
        let (ok, _, err) = post(&g, params.clone());
        assert!(!ok, "{params}");
        let code = err.unwrap().0;
        assert!(
            code == "invalid" || code == "invalid_params",
            "{params}: {code}"
        );
    }
    assert!(read(&g, "read")["messages"].as_array().unwrap().is_empty());
}

#[test]
fn a_credential_in_a_ref_is_refused_over_the_wire_by_the_credentials_rule() {
    let g = gateway();
    let (ok, _, err) = post(
        &g,
        json!({"room":"r","body":"seeking: x","refs":[{"kind":"url","value":"https://u:pw@h.example/x"}]}),
    );
    assert!(!ok);
    let (code, reason) = err.unwrap();
    assert_eq!(code, "refused", "{reason}");
    assert!(!reason.contains("pw@"), "{reason}");
    assert!(read(&g, "read")["messages"].as_array().unwrap().is_empty());
}

#[test]
fn refs_may_come_with_a_scope_and_could_answer_lists_kind_value_and_label_but_no_snippet() {
    let g = gateway();
    let (ok, result, err) = post(
        &g,
        json!({"room":"r","body":"seeking: SECRET-WORDING","scope":"git:foo","refs":[
            {"kind":"pr","value":"#1","label":"fix"},
            {"kind":"log","value":"ci.log","snippet":"SNIPPET-TEXT"}
        ]}),
    );
    assert!(ok, "{err:?}");
    let seq = result.unwrap()["seq"].as_i64().unwrap();
    run(&g, &bob(), &[req("j", "join", json!({"room":"r"}))]);
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
    assert_eq!(
        result["refs"],
        json!([
            {"kind":"pr","value":"#1","label":"fix"},
            {"kind":"log","value":"ci.log"}
        ])
    );
    let text = result.to_string();
    assert!(!text.contains("SECRET-WORDING") && !text.contains("SNIPPET-TEXT"));
    assert_eq!(result["could"], true);
}
