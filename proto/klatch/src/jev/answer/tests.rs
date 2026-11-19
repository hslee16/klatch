use super::*;
use crate::jev::fake::fixtures::*;
use crate::jev::request::{Entry, Key};
use serde_json::{Value, json};

const MODEL: &str = "jev-1.14.0";

fn envelope(answers: Value) -> Value {
    json!({"model": MODEL, "answers": answers, "usage": {"input_tokens": 120, "output_tokens": 4}})
}

fn two_choice() -> Vec<Question> {
    vec![Question::choice(
        "q",
        "Pick",
        &[("a", "first"), ("b", "second")],
    )]
}

fn parse(v: Value) -> Result<Answers, JevError> {
    parse_response(&v.to_string(), MODEL, &two_choice())
}

fn good_answer() -> Value {
    json!({"type": "choice", "choice": "a", "confidence": 0.9, "probabilities": {"a": 0.9, "b": 0.1}})
}

fn judge() -> Vec<Question> {
    vec![Question::choice(
        "verdict",
        "?",
        &[("allow", "a"), ("redact", "r"), ("deny", "d")],
    )]
}

// ---- the recorded and documented shapes ----

#[test]
fn the_recorded_live_choice_reply_parses() {
    let a = parse_response(LIVE_CHOICE, MODEL, &judge()).unwrap();
    let c = a.choice("verdict").unwrap();
    assert_eq!(c.choice, "allow");
    assert!((c.confidence - 0.98).abs() < 1e-9);
    assert!((c.chosen_probability() - 0.99).abs() < 1e-9);
    assert_eq!(a.model(), MODEL);
    assert_eq!(
        a.usage(),
        Some(&Usage {
            input_tokens: Some(120),
            output_tokens: Some(4)
        })
    );
}

#[test]
fn the_recorded_live_noul_reply_parses() {
    let a = parse_response(LIVE_NOUL, MODEL, &[Question::noul("billing", "?")]).unwrap();
    assert_eq!(a.noul("billing"), Some(0.98));
    assert!(
        a.choice("billing").is_none(),
        "an answer is only seen as its own kind"
    );
}

#[test]
fn a_documented_score_reply_parses() {
    let q = Question::score(
        "urgency",
        "?",
        &[
            "Can wait",
            "Needs attention this week",
            "Needs attention today",
        ],
    );
    let a = parse_response(DOCS_SCORE, MODEL, &[q]).unwrap();
    let s = a.score("urgency").unwrap();
    assert_eq!(s.score, 1.7);
    assert_eq!(s.confidence, 0.55);
    assert_eq!(s.probabilities, vec![0.1, 0.1, 0.8]);
    assert_eq!(s.legend.len(), 3);
    assert_eq!(s.most_likely_level(), 2);
}

#[test]
fn a_documented_box_reply_parses_with_or_without_a_confidence() {
    let q = [Question::locate("mug", "the mug")];
    let b = parse_response(DOCS_BOX, MODEL, &q).unwrap();
    let b = b.boxed("mug").unwrap();
    assert_eq!((b.top, b.bottom, b.left, b.right), (0.25, 0.82, 0.44, 0.84));
    assert_eq!(b.confidence, Some(0.92));
    let n = parse_response(DOCS_BOX_NULL_CONFIDENCE, MODEL, &q).unwrap();
    assert_eq!(n.boxed("mug").unwrap().confidence, None);
}

#[test]
fn one_reply_can_hold_every_kind() {
    let qs = vec![
        Question::choice("c", "?", &[("a", "x"), ("b", "y")]),
        Question::noul("n", "?"),
        Question::score("s", "?", &["lo", "hi"]),
        Question::locate("b", "?"),
    ];
    let body = envelope(json!({
        "c": good_answer(),
        "n": {"type": "noul", "noul": 0.1},
        "s": {"type": "score", "score": 0.5, "confidence": 0.0, "legend": {"0": "lo", "1": "hi"}, "probabilities": {"0": 0.5, "1": 0.5}},
        "b": {"type": "box", "box": {"top": 0.1, "bottom": 0.9, "left": 0.1, "right": 0.9}, "confidence": 0.5},
    }));
    let a = parse_response(&body.to_string(), MODEL, &qs).unwrap();
    assert!(
        a.choice("c").is_some()
            && a.noul("n").is_some()
            && a.score("s").is_some()
            && a.boxed("b").is_some()
    );
}

#[test]
fn a_reply_with_a_dashed_option_and_none_parses() {
    let qs = vec![Question::choice_with(
        "route",
        None,
        vec![
            (Key::data("public", "topic-auth"), Entry::Null),
            (Key::Fixed("none"), Entry::Null),
        ],
    )];
    let body = envelope(
        json!({"route": {"type": "choice", "choice": "topic-auth", "confidence": 0.98,
        "probabilities": {"topic-auth": 0.99, "none": 0.01}}}),
    );
    let a = parse_response(&body.to_string(), MODEL, &qs).unwrap();
    assert_eq!(a.choice("route").unwrap().choice, "topic-auth");
}

// ---- usage ----

#[test]
fn usage_is_optional_and_never_needed() {
    let mut e = envelope(json!({"q": good_answer()}));
    e.as_object_mut().unwrap().remove("usage");
    assert_eq!(parse(e).unwrap().usage(), None);
    let mut e = envelope(json!({"q": good_answer()}));
    e["usage"] = json!({});
    assert_eq!(parse(e).unwrap().usage(), Some(&Usage::default()));
    let mut e = envelope(json!({"q": good_answer()}));
    e["usage"] = json!({"input_tokens": -4, "output_tokens": "x", "other": 1});
    assert_eq!(
        parse(e).unwrap().usage(),
        Some(&Usage::default()),
        "an odd count is ignored, not trusted"
    );
    let mut e = envelope(json!({"q": good_answer()}));
    e["usage"] = json!("many");
    assert!(parse(e).is_err());
}

// ---- choice ----

#[test]
fn every_unexpected_choice_shape_fails_closed() {
    let good = good_answer();
    let with = |k: &str, v: Value| {
        let mut g = good_answer();
        g[k] = v;
        envelope(json!({"q": g}))
    };
    let without = |k: &str| {
        let mut g = good_answer();
        g.as_object_mut().unwrap().remove(k);
        envelope(json!({"q": g}))
    };
    let top = |k: &str, v: Option<Value>| {
        let mut e = envelope(json!({"q": good_answer()}));
        match v {
            Some(v) => e[k] = v,
            None => {
                e.as_object_mut().unwrap().remove(k);
            }
        }
        e
    };
    let cases: Vec<Value> = vec![
        json!([]),
        json!("text"),
        json!({}),
        json!({"q": good}), // the old top-level shape, no envelope
        top("model", Some(json!("jev-1.13.0"))),
        top("model", Some(json!("1.14.0"))),
        top("model", Some(json!("jev-latest"))),
        top("model", None),
        top("answers", None),
        top("why", Some(json!("because"))),
        envelope(json!([])),
        envelope(json!({})),
        envelope(json!({"q": good, "extra": good})),
        envelope(json!({"q": "a"})),
        without("type"),
        without("choice"),
        without("probabilities"),
        without("confidence"),
        with("type", json!("noul")),
        with("type", json!("score")),
        with("choice", json!("z")),
        with("choice", json!("b")),
        with("why", json!("x")),
        with("confidence", json!(1.2)),
        with("confidence", json!(-0.1)),
        with("confidence", json!("0.9")),
        with("confidence", json!(null)),
        with("probabilities", json!({"a": 1.5, "b": -0.5})),
        with("probabilities", json!({"a": 0.5, "b": 0.1})),
        with("probabilities", json!({"a": 0.9, "b": 0.1, "c": 0.0})),
        with("probabilities", json!({"a": 1.0})),
        with("probabilities", json!({"a": "0.9", "b": 0.1})),
    ];
    for c in cases {
        assert!(parse(c.clone()).is_err(), "{c}");
    }
    assert!(parse_response("not json", MODEL, &two_choice()).is_err());
    assert!(parse_response("", MODEL, &two_choice()).is_err());
}

#[test]
fn probabilities_may_sum_within_a_small_tolerance() {
    let mut near = good_answer();
    near["probabilities"] = json!({"a": 0.9004, "b": 0.1});
    assert!(parse(envelope(json!({"q": near}))).is_ok());
    near["probabilities"] = json!({"a": 0.92, "b": 0.1});
    assert!(parse(envelope(json!({"q": near}))).is_err());
}

// ---- noul ----

#[test]
fn a_noul_answer_must_be_a_probability() {
    let qs = vec![Question::noul("n", "true?")];
    let p = |x: Value| parse_response(&envelope(json!({"n": x})).to_string(), MODEL, &qs);
    assert_eq!(
        p(json!({"type": "noul", "noul": 0.25})).unwrap().noul("n"),
        Some(0.25)
    );
    for bad in [json!(1.5), json!(-0.1), json!("0.5"), json!(null)] {
        assert!(p(json!({"type": "noul", "noul": bad})).is_err());
    }
    assert!(p(json!({"probability": 0.25})).is_err());
    assert!(p(json!({"noul": 0.25})).is_err());
    assert!(p(json!({"type": "choice", "noul": 0.25})).is_err());
    assert!(p(json!({"type": "noul", "noul": 0.25, "x": 1})).is_err());
}

// ---- score ----

fn score_q() -> Vec<Question> {
    vec![Question::score("s", "?", &["a", "b", "c"])]
}

fn score_ans() -> Value {
    json!({"type": "score", "score": 1.7, "confidence": 0.55,
        "legend": {"0": "a", "1": "b", "2": "c"}, "probabilities": {"0": 0.1, "1": 0.1, "2": 0.8}})
}

fn score(v: Value) -> Result<Answers, JevError> {
    parse_response(&envelope(json!({"s": v})).to_string(), MODEL, &score_q())
}

#[test]
fn every_unexpected_score_shape_fails_closed() {
    assert!(score(score_ans()).is_ok());
    let with = |k: &str, v: Value| {
        let mut g = score_ans();
        g[k] = v;
        g
    };
    let without = |k: &str| {
        let mut g = score_ans();
        g.as_object_mut().unwrap().remove(k);
        g
    };
    let cases = vec![
        without("type"),
        without("score"),
        without("confidence"),
        without("legend"),
        without("probabilities"),
        with("type", json!("choice")),
        with("score", json!(3.0)),
        with("score", json!(-0.1)),
        with("score", json!("1.7")),
        with("confidence", json!(1.5)),
        with("confidence", json!(null)),
        with("legend", json!({"0": "a", "1": "b"})),
        with("legend", json!({"0": "a", "1": "b", "3": "c"})),
        with("legend", json!(["a", "b", "c"])),
        with("probabilities", json!({"0": 0.1, "1": 0.1})),
        with("probabilities", json!({"0": 0.1, "1": 0.1, "3": 0.8})),
        with("probabilities", json!({"0": 0.5, "1": 0.5, "2": 0.5})),
        with("probabilities", json!({"0": -0.5, "1": 0.7, "2": 0.8})),
        with("why", json!("x")),
    ];
    for c in cases {
        assert!(score(c.clone()).is_err(), "{c}");
    }
    // the ends of the scale are allowed
    let mut hi = score_ans();
    hi["score"] = json!(2.0);
    assert!(score(hi).is_ok());
}

#[test]
fn a_legend_that_echoes_structure_is_kept_and_never_shown() {
    let mut a = score_ans();
    a["legend"] = json!({"0": {"description": "SECRET-ECHO"}, "1": ["x"], "2": "c"});
    let ans = score(a).unwrap();
    assert_eq!(ans.score("s").unwrap().legend[1], json!(["x"]));
    assert!(!format!("{ans:?}").contains("SECRET-ECHO"));
}

// ---- box ----

fn box_q() -> Vec<Question> {
    vec![Question::locate("b", "?")]
}

fn box_ans() -> Value {
    json!({"type": "box", "box": {"top": 0.2, "bottom": 0.66, "left": 0.19, "right": 0.68}, "confidence": 0.9})
}

fn boxed(v: Value) -> Result<Answers, JevError> {
    parse_response(&envelope(json!({"b": v})).to_string(), MODEL, &box_q())
}

#[test]
fn every_unexpected_box_shape_fails_closed() {
    assert!(boxed(box_ans()).is_ok());
    let edge = |k: &str, v: Value| {
        let mut g = box_ans();
        g["box"][k] = v;
        g
    };
    let cases = vec![
        edge("top", json!(-0.01)),
        edge("bottom", json!(1.01)),
        edge("left", json!("0.1")),
        edge("right", json!(null)),
        edge("top", json!(0.66)),  // top equals bottom
        edge("left", json!(0.68)), // left equals right
        edge("top", json!(0.9)),   // top below bottom
        edge("right", json!(0.1)), // right left of left
        {
            let mut g = box_ans();
            g["box"].as_object_mut().unwrap().remove("top");
            g
        },
        {
            let mut g = box_ans();
            g["box"]["extra"] = json!(0.5);
            g
        },
        {
            let mut g = box_ans();
            g.as_object_mut().unwrap().remove("confidence");
            g
        },
        {
            let mut g = box_ans();
            g["confidence"] = json!(1.5);
            g
        },
        {
            let mut g = box_ans();
            g["confidence"] = json!("high");
            g
        },
        {
            let mut g = box_ans();
            g["type"] = json!("noul");
            g
        },
        {
            let mut g = box_ans();
            g["why"] = json!(1);
            g
        },
        json!({"type": "box", "box": [0.1, 0.2, 0.3, 0.4], "confidence": null}),
    ];
    for c in cases {
        assert!(boxed(c.clone()).is_err(), "{c}");
    }
    let mut full = box_ans();
    full["box"] = json!({"top": 0.0, "bottom": 1.0, "left": 0.0, "right": 1.0});
    assert!(boxed(full).is_ok());
}
