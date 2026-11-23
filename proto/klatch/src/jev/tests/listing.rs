use super::facilitator::s;
use super::*;

// ---- models ----

#[test]
fn the_listing_is_a_key_check_and_a_model_check() {
    let f = FakeTransport::saying(Ok(fixtures::LIVE_MODELS.to_string()));
    let c = client(&f);
    assert!(c.check_key().is_ok());
    let seen = f.seen();
    assert_eq!((seen[0].method, seen[0].path), (Method::Get, "/v1/models"));
    assert_eq!(seen[0].body, None);
    assert_eq!(seen[0].key, SECRET);
    // the pinned version is not in the list of aliases, which is normal
    assert_eq!(c.check_model().unwrap(), ModelStatus::NotListed);
    let listed = FakeTransport::saying(Ok(
        json!({"models": [{"name": MODEL, "description": "d", "release_date": "2026-10-14"}]})
            .to_string(),
    ));
    assert_eq!(client(&listed).check_model().unwrap(), ModelStatus::Listed);
}

#[test]
fn a_bad_key_is_a_typed_401_and_a_bad_listing_is_refused() {
    let f = FakeTransport::status(401, fixtures::ERR_AUTH);
    assert_eq!(
        client(&f).check_key().unwrap_err(),
        JevError::Unauthorized {
            error_type: Some(ErrorType::Authentication)
        }
    );
    let f = FakeTransport::saying(Ok("{\"models\": 3}".into()));
    assert!(matches!(
        client(&f).check_key().unwrap_err(),
        JevError::Malformed(_)
    ));
}

// ---- the recorded fixtures drive the fake ----

#[test]
fn the_recorded_fixtures_parse_as_what_they_are() {
    let f = FakeTransport::saying(Ok(fixtures::LIVE_NOUL.to_string()));
    let a = client(&f)
        .ask(&state(), &[Question::noul("billing", "?")])
        .unwrap();
    assert_eq!(a.noul("billing"), Some(0.98));
    let f = FakeTransport::saying(Ok(fixtures::DOCS_SCORE.to_string()));
    let q = Question::score("urgency", "?", &["a", "b", "c"]);
    assert_eq!(
        client(&f)
            .ask(&state(), &[q])
            .unwrap()
            .score("urgency")
            .unwrap()
            .score,
        1.7
    );
}

#[test]
fn a_script_replays_in_order_and_then_fails_as_a_network_error() {
    let f = FakeTransport::replaying(vec![Ok(fixtures::LIVE_CHOICE.to_string())]);
    assert!(ask_judge(&f).is_ok());
    assert_eq!(ask_judge(&f).unwrap_err(), JevError::Unreachable);
    assert_eq!(f.calls(), 2);
}

// ---- the decision helpers are exported at the module root ----

#[test]
fn the_decision_helpers_are_reachable_from_the_root() {
    let a = ChoiceAnswer {
        choice: "a".into(),
        probabilities: [("a".to_string(), 0.95), ("none".to_string(), 0.05)]
            .into_iter()
            .collect(),
        confidence: 0.9,
    };
    assert_eq!(pick(&s(&["a"]), None, &a, 0.9), Some("a".into()));
    assert_eq!(NoulThresholds::default().band(0.9), Band::Yes);
    assert_eq!(NONE_CHOICE, "none");
    assert_eq!(REDACTED_PLACEHOLDER, "[redacted by the judge]");
    assert!(top_to_second_ratio(&a).unwrap() > 1.0);
    assert!(choice_confidence(&[0.5, 0.5]).abs() < 1e-9 && noul_confidence(0.5).abs() < 1e-9);
    assert!(score_confidence(&[0.0, 1.0]) > 0.99);
}
