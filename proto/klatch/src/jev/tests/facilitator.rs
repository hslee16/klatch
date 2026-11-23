use super::*;

// ---- the facilitator's question ----

fn l(label: &str, value: &str) -> Labelled {
    Labelled {
        label: label.into(),
        value: value.into(),
    }
}

pub(super) fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn route(
    ceiling: Option<&str>,
    sender: &Labelled,
    topic: &Labelled,
    cands: &[Labelled],
) -> Option<(RouteQuestion, Vec<String>)> {
    route_question(&LabelChain::default(), ceiling, sender, topic, cands)
}

#[test]
fn facts_at_or_above_the_ceiling_are_never_in_the_question() {
    let cands = [
        l("public", "topic-api"),
        l("internal", "topic-secret-x"),
        l("private", "topic-y"),
        l("", "topic-z"),
    ];
    let (q, sent) = route(
        Some("internal"),
        &l("public", "alice"),
        &l("public", "src/api"),
        &cands,
    )
    .unwrap();
    assert_eq!(sent, s(&["topic-api"]));
    let f = FakeTransport::saying(Ok(json!({}).to_string()));
    let _ = client(&f).ask(&q.state, std::slice::from_ref(&q.question));
    let body = f.last_body().unwrap();
    for leaked in ["topic-secret-x", "topic-y", "topic-z"] {
        assert!(!body.contains(leaked), "{leaked}");
    }
    assert!(body.contains("topic-api") && body.contains("alice") && body.contains("src/api"));
}

#[test]
fn the_client_gate_checks_the_labels_again_even_if_the_question_builder_were_wrong() {
    // a question built under a higher ceiling is refused by a client whose gate is lower
    let cands = [l("internal", "topic-a")];
    let (q, _) = route(
        Some("private"),
        &l("public", "a"),
        &l("public", "t"),
        &cands,
    )
    .unwrap();
    let f = FakeTransport::saying(Ok(json!({}).to_string()));
    let e = client(&f)
        .ask(&q.state, std::slice::from_ref(&q.question))
        .unwrap_err();
    assert_eq!(e, JevError::Refused(RequestError::Gated));
    assert_eq!(f.calls(), 0);
}

#[test]
fn a_gated_sender_or_topic_means_no_question_at_all() {
    let cands = [l("public", "topic-api")];
    assert!(
        route(
            Some("internal"),
            &l("internal", "alice"),
            &l("public", "t"),
            &cands
        )
        .is_none()
    );
    assert!(
        route(
            Some("internal"),
            &l("public", "alice"),
            &l("secret", "t"),
            &cands
        )
        .is_none()
    );
    assert!(route(None, &l("public", "alice"), &l("public", "t"), &cands).is_none());
    assert!(
        route(
            Some("nonsense"),
            &l("public", "a"),
            &l("public", "t"),
            &cands
        )
        .is_none()
    );
}

#[test]
fn no_sendable_candidate_means_no_question() {
    let cands = [l("internal", "topic-a")];
    assert!(
        route(
            Some("internal"),
            &l("public", "a"),
            &l("public", "t"),
            &cands
        )
        .is_none()
    );
}

#[test]
fn a_candidate_named_none_is_dropped_and_repeats_collapse() {
    let cands = [
        l("public", "none"),
        l("public", "topic-a"),
        l("public", "topic-a"),
    ];
    let (_, sent) = route(
        Some("internal"),
        &l("public", "a"),
        &l("public", "t"),
        &cands,
    )
    .unwrap();
    assert_eq!(sent, s(&["topic-a"]));
}

#[test]
fn a_routing_reply_with_a_dashed_candidate_and_none_is_understood() {
    let cands = [l("public", "topic-auth")];
    let (q, offered) = route(
        Some("internal"),
        &l("public", "a"),
        &l("public", "t"),
        &cands,
    )
    .unwrap();
    let reply = envelope(
        json!({"route": {"type": "choice", "choice": "topic-auth", "confidence": 0.98,
        "probabilities": {"topic-auth": 0.99, "none": 0.01}}}),
    );
    let f = FakeTransport::saying(Ok(reply.to_string()));
    let a = client(&f)
        .ask(&q.state, std::slice::from_ref(&q.question))
        .unwrap();
    let c = a.choice(ROUTE_QUESTION_ID).unwrap();
    assert_eq!(pick(&offered, None, c, 0.9), Some("topic-auth".into()));
}
