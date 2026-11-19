use super::*;
use crate::judge_eval::corpus::Truth;
use crate::judge_eval::judges::Judged;
use klatch::domain::{ReviewItem, Verdict};
use std::cell::RefCell;
use std::rc::Rc;

/// Answers with a canned `Judged` and counts its calls.
struct Fake {
    name: &'static str,
    judged: Judged,
    calls: Rc<RefCell<usize>>,
}
impl EvalJudge for Fake {
    fn name(&self) -> String {
        self.name.into()
    }
    fn evaluate(&self, _: &ReviewItem) -> Judged {
        *self.calls.borrow_mut() += 1;
        self.judged.clone()
    }
}

fn review() -> ReviewItem {
    ReviewItem {
        id: 1,
        room: "judge-eval".into(),
        seq: 1,
        to_label: "public".into(),
        from_label: "public".into(),
        reason: "crossing".into(),
        text: "hello".into(),
    }
}

fn first(pick: Truth, probability: f64, confidence: f64) -> (Fake, Rc<RefCell<usize>>) {
    let calls = Rc::new(RefCell::new(0));
    let verdict = match pick {
        Truth::Allow => Verdict::Allow,
        Truth::Redact => Verdict::Redact("[x]".into()),
        Truth::Deny => Verdict::Deny("x".into()),
    };
    (
        Fake {
            name: "jev:jev-1.14.0",
            judged: Judged {
                raw: Some((pick, confidence)),
                probability: Some(probability),
                tokens_in: Some(100),
                tokens_out: Some(4),
                ..Judged::of(verdict)
            },
            calls: calls.clone(),
        },
        calls,
    )
}

fn second(verdict: Verdict, cost: Option<f64>) -> (Fake, Rc<RefCell<usize>>) {
    let calls = Rc::new(RefCell::new(0));
    (
        Fake {
            name: "claude:sonnet",
            judged: Judged {
                tokens_in: Some(24000),
                tokens_out: Some(40),
                cost_usd: cost,
                ..Judged::of(verdict)
            },
            calls: calls.clone(),
        },
        calls,
    )
}

fn cascade(f: Fake, s: Fake) -> CascadeEval {
    CascadeEval::new(Box::new(f), Box::new(s), CascadePolicy::default())
}

#[test]
fn a_confident_deny_is_decided_by_the_first_stage_and_the_second_is_not_called() {
    let (f, _) = first(Truth::Deny, 0.97, 0.9);
    let (s, s_calls) = second(Verdict::Allow, Some(0.01));
    let j = cascade(f, s).evaluate(&review());
    assert!(matches!(j.verdict, Verdict::Deny(_)));
    assert_eq!(j.stage.as_deref(), Some("first"));
    assert_eq!(*s_calls.borrow(), 0);
    assert_eq!(j.tokens_in, Some(100));
    assert_eq!(j.cost_usd, None, "nothing reported a cost");
}

#[test]
fn an_escalated_item_is_decided_by_the_second_stage_and_costs_are_summed() {
    let (f, f_calls) = first(Truth::Allow, 0.7, 0.4);
    let (s, s_calls) = second(Verdict::Deny("no".into()), Some(0.0123));
    let j = cascade(f, s).evaluate(&review());
    assert!(matches!(j.verdict, Verdict::Deny(_)));
    assert_eq!(j.stage.as_deref(), Some("second"));
    assert_eq!((*f_calls.borrow(), *s_calls.borrow()), (1, 1));
    assert_eq!(j.tokens_in, Some(24100));
    assert_eq!(j.tokens_out, Some(44));
    assert_eq!(j.cost_usd, Some(0.0123));
}

#[test]
fn every_first_stage_redact_is_escalated() {
    for (p, c) in [(1.0, 1.0), (0.99, 0.5), (0.3, 0.0)] {
        let (f, _) = first(Truth::Redact, p, c);
        let (s, s_calls) = second(Verdict::Redact("clean".into()), None);
        let j = cascade(f, s).evaluate(&review());
        assert_eq!(j.verdict, Verdict::Redact("clean".into()));
        assert_eq!(j.stage.as_deref(), Some("second"));
        assert_eq!(*s_calls.borrow(), 1);
    }
}

#[test]
fn a_second_stage_that_fails_defers_and_says_why_and_never_falls_back() {
    let (f, _) = first(Truth::Allow, 0.9, 0.9);
    let calls = Rc::new(RefCell::new(0));
    let s = Fake {
        name: "claude:sonnet",
        judged: Judged::failed("could not run claude".into()),
        calls,
    };
    let j = cascade(f, s).evaluate(&review());
    assert_eq!(j.verdict, Verdict::Defer);
    assert_eq!(j.stage.as_deref(), Some("second"));
    assert!(j.error.unwrap().contains("could not run claude"));
}

#[test]
fn a_first_stage_with_no_pick_escalates() {
    let calls = Rc::new(RefCell::new(0));
    let f = Fake {
        name: "jev:jev-1.14.0",
        judged: Judged::failed("no usable answer".into()),
        calls,
    };
    let (s, _) = second(Verdict::Allow, None);
    let j = cascade(f, s).evaluate(&review());
    assert_eq!(j.verdict, Verdict::Allow);
    assert_eq!(j.stage.as_deref(), Some("second"));
    assert_eq!(
        j.error, None,
        "the second stage decided, so no failed call is reported"
    );
}

#[test]
fn the_confidence_stands_in_for_a_probability_the_first_judge_does_not_report() {
    // a claude-like first stage: a confidence only
    let calls = Rc::new(RefCell::new(0));
    let f = Fake {
        name: "claude:haiku",
        judged: Judged {
            raw: Some((Truth::Deny, 0.95)),
            ..Judged::of(Verdict::Deny("x".into()))
        },
        calls,
    };
    let (s, _) = second(Verdict::Allow, None);
    let j = cascade(f, s).evaluate(&review());
    assert_eq!(j.stage.as_deref(), Some("first"));
}

#[test]
fn the_name_lists_both_stages() {
    let (f, _) = first(Truth::Deny, 1.0, 1.0);
    let (s, _) = second(Verdict::Allow, None);
    assert_eq!(cascade(f, s).name(), "cascade:jev:jev-1.14.0,claude:sonnet");
}

// the report

fn record(
    id: &str,
    truth: Truth,
    verdict: crate::judge_eval::metrics::Called,
    stage: &str,
) -> crate::judge_eval::metrics::Record {
    crate::judge_eval::metrics::Record {
        id: id.into(),
        category: "everyday-chat".into(),
        injection: false,
        truth,
        verdict,
        redaction_leaked: false,
        redaction_data_leaked: false,
        contested: false,
        raw: None,
        probability: None,
        stage: Some(stage.into()),
        latency_ms: 10.0,
        tokens_in: None,
        tokens_out: None,
        cost_usd: Some(0.5),
        error: None,
    }
}

#[test]
fn the_report_gives_the_escalation_rate_and_the_stage_of_every_row() {
    use crate::judge_eval::metrics::{Called, summarize};
    let records = vec![
        record("a", Truth::Deny, Called::Deny, "first"),
        record("b", Truth::Allow, Called::Allow, "second"),
        record("c", Truth::Allow, Called::Allow, "second"),
        record("d", Truth::Deny, Called::Deny, "second"),
    ];
    let s = summarize(&records, 0.01);
    assert_eq!((s.staged, s.escalated), (4, 3));
    let text = crate::judge_eval::report::format(&s, "cascade:x,y", "dev");
    assert!(
        text.contains("escalated to the second judge: 3 of 4"),
        "{text}"
    );
    assert!(text.contains("75.0%"), "{text}");
    let json = crate::judge_eval::report::to_json(&s, &records, "cascade:x,y", "dev");
    assert_eq!(json["items"][0]["stage"], "first");
    assert_eq!(json["items"][1]["stage"], "second");
    assert_eq!(json["summary"]["escalated"], 3);
    assert_eq!(json["summary"]["escalation_rate"], 0.75);
}

#[test]
fn a_judge_with_no_stages_has_no_escalation_line() {
    use crate::judge_eval::metrics::{Called, summarize};
    let mut r = record("a", Truth::Allow, Called::Allow, "first");
    r.stage = None;
    let s = summarize(&[r], 0.01);
    assert_eq!(s.staged, 0);
    let text = crate::judge_eval::report::format(&s, "rules", "dev");
    assert!(!text.contains("escalated"), "{text}");
}
