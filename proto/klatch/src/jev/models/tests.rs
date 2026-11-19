use super::*;
use crate::jev::fake::fixtures::LIVE_MODELS;
use serde_json::json;

#[test]
fn the_recorded_listing_parses() {
    let l = parse_model_list(LIVE_MODELS).unwrap();
    let names: Vec<&str> = l.models().iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, vec!["jev-latest", "jev-preview"]);
    assert_eq!(l.models()[0].release_date, "2026-09-15");
    assert!(l.contains("jev-latest"));
    assert!(
        !l.contains("jev-1.14.0"),
        "versioned ids are accepted whether or not they are listed"
    );
}

#[test]
fn an_empty_list_is_a_working_key_with_nothing_listed() {
    assert!(
        parse_model_list(r#"{"models":[]}"#)
            .unwrap()
            .models()
            .is_empty()
    );
}

#[test]
fn every_unexpected_shape_is_refused() {
    let m = |v: serde_json::Value| parse_model_list(&v.to_string()).is_err();
    let ok = json!({"name": "jev-latest", "description": "d", "release_date": "2026-09-15"});
    assert!(!m(json!({"models": [ok.clone()]})));
    assert!(parse_model_list("not json").is_err());
    assert!(parse_model_list("[]").is_err());
    assert!(m(json!({})));
    assert!(m(json!({"models": {}})));
    assert!(m(json!({"models": [ok.clone()], "extra": 1})));
    assert!(
        m(json!({"models": [ok.clone(), ok.clone()]})),
        "a name twice"
    );
    assert!(m(json!({"models": ["jev-latest"]})));
    for bad in [
        json!({"name": "", "description": "d", "release_date": "2026-09-15"}),
        json!({"name": "x", "release_date": "2026-09-15"}),
        json!({"name": "x", "description": 1, "release_date": "2026-09-15"}),
        json!({"name": "x", "description": "d", "release_date": "15 Sep 2026"}),
        json!({"name": "x", "description": "d", "release_date": "2026-9-15"}),
        json!({"name": "x", "description": "d", "release_date": "2026-13-45"}),
        json!({"name": "x", "description": "d", "release_date": "2026-09-15", "extra": 1}),
        json!({"name": 5, "description": "d", "release_date": "2026-09-15"}),
    ] {
        assert!(m(json!({"models": [bad.clone()]})), "{bad}");
    }
}
