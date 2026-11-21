//! The screened routing advice against the fake transport (#917): nothing here touches a network.

use super::*;
use crate::jev::fake::FakeTransport;
use crate::jev::{Gate, JevClient, JevConfig, Labelled, NoulThresholds};
use serde_json::{Value, json};

const MODEL: &str = "jev-1.14.0";
const SECRET: &str = "sk-live-SECRET-1234";

fn gate() -> Gate {
    Gate::new(LabelChain::default(), Some("internal"))
}

fn client(f: &FakeTransport) -> JevClient<&FakeTransport> {
    JevClient::new(f, JevConfig::new(MODEL, 0.8).unwrap())
        .with_key_source(Box::new(|| Some(SECRET.to_string())))
        .with_gate(gate())
}

fn l(label: &str, value: &str) -> Labelled {
    Labelled {
        label: label.into(),
        value: value.into(),
    }
}

fn cands() -> Vec<Labelled> {
    vec![l("public", "topic-api"), l("public", "topic-docs")]
}

/// A reply with a route choice (probability and confidence `p`) and a screen noul `screen`.
fn reply(choice: &str, p: f64, screen: f64) -> String {
    let rest = (1.0 - p) / 2.0;
    let mut probs = json!({"topic-api": rest, "topic-docs": rest, "none": rest});
    probs[choice] = json!(p);
    let a = json!({
        "route": {"type": "choice", "choice": choice, "confidence": p, "probabilities": probs},
        "needs_route": {"type": "noul", "noul": screen},
    });
    json!({"model": MODEL, "answers": a, "usage": {"input_tokens": 50, "output_tokens": 4}})
        .to_string()
}

fn sender() -> Labelled {
    l("public", "alice")
}

fn topic() -> Labelled {
    l("public", "src/api")
}

fn advise(f: &FakeTransport, cs: &[Labelled], rules: Option<&str>) -> Option<String> {
    advise_route(
        &client(f),
        NoulThresholds::default(),
        &LabelChain::default(),
        Some("internal"),
        &sender(),
        &topic(),
        cs,
        rules,
    )
}

#[test]
fn a_clear_yes_lets_the_confident_pick_run() {
    let f = FakeTransport::saying(Ok(reply("topic-docs", 0.95, 0.95)));
    assert_eq!(
        advise(&f, &cands(), Some("topic-api")).as_deref(),
        Some("topic-docs")
    );
    assert_eq!(f.calls(), 1, "one request holds both questions");
    let body: Value = serde_json::from_str(&f.last_body().unwrap()).unwrap();
    assert!(body["questions"]["route"].is_object());
    assert!(body["questions"]["needs_route"].is_object());
}

#[test]
fn a_clear_no_narrows_to_nothing_and_unsure_keeps_the_rules() {
    let f = FakeTransport::saying(Ok(reply("topic-docs", 0.95, 0.05)));
    assert_eq!(advise(&f, &cands(), Some("topic-api")), None);
    let f = FakeTransport::saying(Ok(reply("topic-docs", 0.95, 0.5)));
    assert_eq!(
        advise(&f, &cands(), Some("topic-api")).as_deref(),
        Some("topic-api")
    );
}

#[test]
fn it_can_never_name_a_recipient_the_rules_and_guard_did_not_offer() {
    let f = FakeTransport::saying(Ok(reply("topic-secret", 0.97, 0.97)));
    assert_eq!(
        advise(&f, &cands(), Some("topic-api")).as_deref(),
        Some("topic-api")
    );
}

#[test]
fn gated_facts_are_never_sent_and_a_gated_sender_means_no_call() {
    let f = FakeTransport::saying(Ok(reply("topic-api", 0.95, 0.95)));
    let cs = vec![l("public", "topic-api"), l("internal", "topic-hidden")];
    advise(&f, &cs, None);
    assert!(!f.last_body().unwrap().contains("topic-hidden"));

    let f = FakeTransport::saying(Ok(reply("topic-api", 0.95, 0.95)));
    let got = advise_route(
        &client(&f),
        NoulThresholds::default(),
        &LabelChain::default(),
        Some("internal"),
        &l("internal", "alice"),
        &topic(),
        &cands(),
        Some("topic-docs"),
    );
    assert_eq!(got.as_deref(), Some("topic-docs"));
    assert_eq!(f.calls(), 0);
}

#[test]
fn every_failure_leaves_the_rules_decision() {
    let f = FakeTransport::status(500, "{}");
    assert_eq!(
        advise(&f, &cands(), Some("topic-api")).as_deref(),
        Some("topic-api")
    );
    let f = FakeTransport::saying(Ok("not json".into()));
    assert_eq!(advise(&f, &cands(), None), None);
    // a reply missing the screen answer is not a yes
    let only_route = json!({"model": MODEL, "answers": {"route": {"type": "choice",
        "choice": "topic-docs", "confidence": 0.95,
        "probabilities": {"topic-api": 0.02, "topic-docs": 0.95, "none": 0.03}}}})
    .to_string();
    let f = FakeTransport::saying(Ok(only_route));
    assert_eq!(
        advise(&f, &cands(), Some("topic-api")).as_deref(),
        Some("topic-api")
    );
}

#[test]
fn a_missing_key_sends_nothing_and_leaves_the_rules() {
    let f = FakeTransport::saying(Ok(reply("topic-docs", 0.95, 0.95)));
    let c = JevClient::new(&f, JevConfig::new(MODEL, 0.8).unwrap())
        .with_key_source(Box::new(|| None))
        .with_gate(gate());
    let got = advise_route(
        &c,
        NoulThresholds::default(),
        &LabelChain::default(),
        Some("internal"),
        &sender(),
        &topic(),
        &cands(),
        Some("topic-api"),
    );
    assert_eq!(got.as_deref(), Some("topic-api"));
    assert_eq!(f.calls(), 0);
}
