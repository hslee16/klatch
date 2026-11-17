use super::*;

// ---- stored attribution and label are reported, not placeholders (#139) ----

#[test]
fn read_and_the_stream_report_the_stored_attribution_and_label() {
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .post_as(
            &NewMessage::new(
                ("r", "carol"),
                crate::domain::Attribution::Verified,
                "internal",
                "proven",
            ),
            1,
        )
        .unwrap();
    store
        .post_as(
            &NewMessage::new(
                ("r", "eve"),
                crate::domain::Attribution::Unattributed,
                "public",
                "unproven",
            ),
            2,
        )
        .unwrap();
    let g = Gateway::new(store, FixedClock(5), AllowAll);
    let f = run(
        &g,
        &bob(),
        &[
            req("r", "read", json!({"room":"r","peek":true})),
            req("s", "subscribe", json!({"room":"r","from_seq":0})),
        ],
    );
    let read = response(&f, "r").1.unwrap();
    let m = read["messages"].as_array().unwrap();
    assert_eq!(
        (m[0]["attribution"].as_str(), m[0]["label"].as_str()),
        (Some("verified"), Some("internal"))
    );
    assert_eq!(
        (m[1]["attribution"].as_str(), m[1]["label"].as_str()),
        (Some("unattributed"), Some("public"))
    );
    let streamed: Vec<_> = f
        .iter()
        .filter_map(|x| match x {
            ServerFrame::Message {
                attribution, label, ..
            } => Some((attribution.clone(), label.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        streamed,
        vec![
            (Attribution::Verified, "internal".to_string()),
            (Attribution::Unattributed, "public".to_string())
        ]
    );
}

#[test]
fn a_message_posted_through_a_verified_session_reads_back_verified() {
    let g = gateway();
    run(
        &g,
        &Principal::verified("carol"),
        &[req("p", "post", json!({"room":"r","body":"hello"}))],
    );
    let f = run(&g, &bob(), &[req("r", "read", json!({"room":"r"}))]);
    assert_eq!(
        response(&f, "r").1.unwrap()["messages"][0]["attribution"],
        "verified"
    );
}

#[test]
fn an_empty_post_body_is_an_invalid_request_over_the_wire() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[req("p", "post", json!({"room":"r","body":"   "}))],
    );
    assert_eq!(response(&f, "p").2.unwrap().0, "invalid");
}

#[test]
fn status_is_set_by_the_caller_and_shown_by_who() {
    let g = gateway();
    run(
        &g,
        &alice(),
        &[req("j", "post", json!({"room":"r","body":"hi"}))],
    );
    let f = run(
        &g,
        &alice(),
        &[
            req("s", "status", json!({"room":"r","status":"done leaving"})),
            req("w", "who", json!({"room":"r"})),
        ],
    );
    assert!(response(&f, "s").0, "status failed");
    let members = response(&f, "w").1.unwrap()["members"].clone();
    let a = members
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "alice")
        .unwrap();
    assert_eq!(a["status"], "done leaving");
}

#[test]
fn a_bad_status_is_an_invalid_error() {
    assert_invalid_request("status", json!({"room":"r","status":""}));
}

#[test]
fn scopes_let_two_members_claim_the_same_path_and_a_shared_claim_reaches_every_scope() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[
            req("s", "scope", json!({"scope":"git:a"})),
            req("c", "claim", json!({"room":"r","path":"src/lib.rs"})),
        ],
    );
    assert!(response(&f, "s").0 && response(&f, "c").0);
    let f = run(
        &g,
        &bob(),
        &[
            req("s", "scope", json!({"scope":"git:b"})),
            req("c", "claim", json!({"room":"r","path":"src/lib.rs"})),
            req(
                "d",
                "claim",
                json!({"room":"r","path":"docs","shared":true}),
            ),
            req("l", "claims", json!({"room":"r"})),
        ],
    );
    assert!(
        response(&f, "c").0,
        "a different scope may claim the same path"
    );
    assert!(response(&f, "d").0);
    let claims = response(&f, "l").1.unwrap()["claims"].clone();
    let scopes: Vec<_> = claims
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["scope"].as_str().unwrap_or("").to_string())
        .collect();
    assert!(
        scopes.contains(&"git:a".to_string())
            && scopes.contains(&"git:b".to_string())
            && scopes.contains(&"*".to_string()),
        "{scopes:?}"
    );
    let f = run(
        &g,
        &alice(),
        &[req("x", "claim", json!({"room":"r","path":"docs/api.md"}))],
    );
    let (ok, _, err) = response(&f, "x");
    assert!(!ok);
    assert_eq!(err.unwrap().0, "conflict");
}

#[test]
fn a_bad_scope_is_an_invalid_error_over_the_wire() {
    assert_invalid_request("scope", json!({"scope":"has space"}));
}

// ---- typed handoffs on the wire (#123) ----

#[test]
fn a_post_with_a_kind_and_payload_reaches_a_subscriber_with_both_and_a_plain_post_without() {
    let g = gateway();
    let f = run(
        &g,
        &bob(),
        &[
            req(
                "a",
                "post",
                json!({"room":"r","body":"see the order","kind":"work-order","payload":"{\"id\":7}"}),
            ),
            req("b", "post", json!({"room":"r","body":"just words"})),
        ],
    );
    assert!(response(&f, "a").0 && response(&f, "b").0);
    let f = run(
        &g,
        &alice(),
        &[req("s", "subscribe", json!({"room":"r","from_seq":0}))],
    );
    let handoffs: Vec<(Option<String>, Option<String>)> = f
        .iter()
        .filter_map(|x| match x {
            ServerFrame::Message { kind, payload, .. } => Some((kind.clone(), payload.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        handoffs,
        vec![
            (
                Some("work-order".to_string()),
                Some("{\"id\":7}".to_string())
            ),
            (None, None)
        ]
    );
}

#[test]
fn a_payload_without_a_kind_a_bad_kind_and_a_payload_the_rules_refuse_are_all_failed_responses() {
    let g = gateway();
    let f = run(
        &g,
        &bob(),
        &[
            req(
                "nokind",
                "post",
                json!({"room":"r","body":"b","payload":"x"}),
            ),
            req(
                "badkind",
                "post",
                json!({"room":"r","body":"b","kind":"has space","payload":"x"}),
            ),
            req(
                "secret",
                "post",
                json!({"room":"r","body":"b","kind":"plan","payload":"ghp_abcdefghijklmnopqrstuvwxyz0123456789"}),
            ),
        ],
    );
    for id in ["nokind", "badkind", "secret"] {
        assert!(!response(&f, id).0, "{id} was accepted");
    }
    let text = format!("{f:?}");
    assert!(!text.contains("ghp_abc"), "a refusal repeated the payload");
    assert!(g.tail(&alice(), "r", (0, 10)).unwrap().is_empty());
}

#[test]
fn read_over_the_wire_carries_a_handoff_and_leaves_an_ordinary_message_as_it_was() {
    let g = gateway();
    g.post_handoff(
        &bob(),
        ("r", "see", None),
        &crate::gateway::Handoff {
            kind: "plan",
            payload: Some("P"),
        },
    )
    .unwrap();
    g.post(&bob(), "r", "plain").unwrap();
    let f = run(
        &g,
        &alice(),
        &[req("rd", "read", json!({"room":"r","peek":true}))],
    );
    let result = response(&f, "rd").1.unwrap();
    let ms = result["messages"].as_array().unwrap();
    assert_eq!(
        (ms[0]["kind"].as_str(), ms[0]["payload"].as_str()),
        (Some("plan"), Some("P"))
    );
    assert!(
        ms[1].get("kind").is_none() && ms[1].get("payload").is_none(),
        "{}",
        ms[1]
    );
}
