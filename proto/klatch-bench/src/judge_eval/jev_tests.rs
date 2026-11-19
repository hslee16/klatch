use super::*;
use crate::judge_eval::corpus::Truth;
use crate::judge_eval::judges::EvalJudge;
use klatch::domain::{HostedConfig, HostedJudge, Judge, LabelChain, ReviewItem, Verdict};
use klatch::jev::{ApiKey, JevClient, JevConfig, JevJudge, JevTransport, JevTransportError};
use std::cell::RefCell;

const MODEL: &str = "jev-1.14.0";

struct Fake {
    calls: RefCell<usize>,
    reply: Result<String, JevTransportError>,
}
impl JevTransport for &Fake {
    fn post(&self, _key: &ApiKey, _body: &str) -> Result<String, JevTransportError> {
        *self.calls.borrow_mut() += 1;
        self.reply.clone()
    }
}

fn reply(choice: &str, conf: f64) -> String {
    let rest = (1.0 - conf) / 2.0;
    let mut p = serde_json::json!({"allow": rest, "redact": rest, "deny": rest});
    p[choice] = serde_json::json!(conf);
    serde_json::json!({
        "model": MODEL,
        "answers": {"verdict": {"type": "choice", "choice": choice, "confidence": conf, "probabilities": p}},
        "usage": {"input_tokens": 120, "output_tokens": 4}
    })
    .to_string()
}

fn review(label: &str) -> ReviewItem {
    ReviewItem {
        id: 1,
        room: "judge-eval".into(),
        seq: 1,
        to_label: "public".into(),
        from_label: label.into(),
        reason: "crossing".into(),
        text: "hello".into(),
    }
}

fn key() -> Box<dyn Fn() -> Option<String>> {
    Box::new(|| Some("k".to_string()))
}

fn eval<'a>(f: &'a Fake, threshold: f64, ceiling: &str) -> JevEval<&'a Fake> {
    JevEval::new(f, JevConfig::new(MODEL, threshold).unwrap(), ceiling, key())
}

/// The production path, for comparison: `HostedJudge<JevJudge>`.
fn production(f: &Fake, threshold: f64) -> HostedJudge<JevJudge<&Fake>> {
    HostedJudge::new(
        JevJudge::new(
            JevClient::new(f, JevConfig::new(MODEL, threshold).unwrap()).with_key_source(key()),
        ),
        LabelChain::default(),
        HostedConfig {
            enabled: true,
            max_label: Some("secret".into()),
        },
    )
}

#[test]
fn the_verdict_is_the_production_one_and_the_raw_pick_and_confidence_are_kept() {
    for (choice, conf, threshold) in [
        ("allow", 0.95, 0.8),
        ("redact", 0.9, 0.8),
        ("deny", 0.85, 0.8),
        ("allow", 0.6, 0.8),
        ("deny", 0.5, 0.8),
    ] {
        let f = Fake {
            calls: RefCell::new(0),
            reply: Ok(reply(choice, conf)),
        };
        let out = eval(&f, threshold, "secret").evaluate(&review("private"));
        assert_eq!(
            out.verdict,
            production(&f, threshold).judge(&review("private")),
            "{choice} {conf}"
        );
        let pick = Truth::parse(choice).unwrap();
        assert_eq!(out.raw, Some((pick, conf)));
        assert_eq!(out.tokens_in, Some(120));
        assert_eq!(out.tokens_out, Some(4));
    }
}

#[test]
fn a_transport_failure_or_a_missing_key_defers_with_a_reason() {
    let f = Fake {
        calls: RefCell::new(0),
        reply: Err(JevTransportError::Network),
    };
    let out = eval(&f, 0.8, "secret").evaluate(&review("public"));
    assert_eq!(out.verdict, Verdict::Defer);
    assert!(out.raw.is_none());
    assert!(out.error.is_some());
    let f = Fake {
        calls: RefCell::new(0),
        reply: Ok(reply("allow", 0.9)),
    };
    let j = JevEval::new(
        &f,
        JevConfig::new(MODEL, 0.8).unwrap(),
        "secret",
        Box::new(|| None),
    );
    let out = j.evaluate(&review("public"));
    assert_eq!(out.verdict, Verdict::Defer);
    assert_eq!(*f.calls.borrow(), 0);
}

#[test]
fn the_label_gate_holds_and_the_name_says_jev() {
    let f = Fake {
        calls: RefCell::new(0),
        reply: Ok(reply("allow", 0.9)),
    };
    let j = eval(&f, 0.8, "internal");
    assert_eq!(j.evaluate(&review("internal")).verdict, Verdict::Defer);
    assert_eq!(*f.calls.borrow(), 0);
    assert_eq!(j.evaluate(&review("public")).verdict, Verdict::Allow);
    assert_eq!(j.name(), "jev:jev-1.14.0");
}
