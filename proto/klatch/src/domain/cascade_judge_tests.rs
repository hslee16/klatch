use super::*;
use std::cell::RefCell;

struct FakeReader {
    reading: Option<Reading>,
    asked: RefCell<usize>,
}
impl FakeReader {
    fn says(lean: Lean, probability: f64, confidence: f64) -> Self {
        FakeReader {
            reading: Some(Reading {
                lean,
                probability,
                confidence,
            }),
            asked: RefCell::new(0),
        }
    }
    fn nothing() -> Self {
        FakeReader {
            reading: None,
            asked: RefCell::new(0),
        }
    }
}
impl Reader for &FakeReader {
    fn read(&self, _: &ReviewItem) -> Option<Reading> {
        *self.asked.borrow_mut() += 1;
        self.reading
    }
}

struct FakeSecond {
    verdict: Verdict,
    asked: RefCell<usize>,
}
impl FakeSecond {
    fn says(verdict: Verdict) -> Self {
        FakeSecond {
            verdict,
            asked: RefCell::new(0),
        }
    }
}
impl Judge for &FakeSecond {
    fn judge(&self, _: &ReviewItem) -> Verdict {
        *self.asked.borrow_mut() += 1;
        self.verdict.clone()
    }
}

fn item(label: &str) -> ReviewItem {
    ReviewItem {
        id: 1,
        room: "r".into(),
        seq: 1,
        to_label: "public".into(),
        from_label: label.into(),
        reason: "crossing".into(),
        text: "the text".into(),
    }
}

fn run(first: &FakeReader, second: &FakeSecond, policy: CascadePolicy) -> Decided {
    CascadeJudge::new(first, second, policy).decide(&item("public"))
}

#[test]
fn a_confident_deny_is_taken_without_the_second_judge() {
    let (f, s) = (
        FakeReader::says(Lean::Deny, 0.95, 0.9),
        FakeSecond::says(Verdict::Allow),
    );
    let d = run(&f, &s, CascadePolicy::default());
    assert!(matches!(d.verdict, Verdict::Deny(_)));
    assert_eq!(d.stage, Stage::First);
    assert_eq!(*s.asked.borrow(), 0);
}

#[test]
fn a_deny_below_the_threshold_escalates_and_the_second_verdict_is_final() {
    let (f, s) = (
        FakeReader::says(Lean::Deny, 0.6, 0.2),
        FakeSecond::says(Verdict::Allow),
    );
    let d = run(&f, &s, CascadePolicy::default());
    assert_eq!(d.verdict, Verdict::Allow);
    assert_eq!(d.stage, Stage::Second);
}

#[test]
fn an_allow_needs_both_the_probability_and_the_confidence() {
    let policy = CascadePolicy::default();
    let s = FakeSecond::says(Verdict::Deny("no".into()));
    let sure = FakeReader::says(Lean::Allow, 0.99, 0.99);
    assert_eq!(run(&sure, &s, policy).verdict, Verdict::Allow);
    assert_eq!(*s.asked.borrow(), 0);
    for (p, c) in [(0.99, 0.5), (0.5, 0.99), (0.9, 0.9)] {
        let f = FakeReader::says(Lean::Allow, p, c);
        let d = run(&f, &s, policy);
        assert_eq!(d.stage, Stage::Second, "{p} {c}");
        assert!(matches!(d.verdict, Verdict::Deny(_)));
    }
}

#[test]
fn an_allow_is_held_to_a_higher_bar_than_a_deny_by_default() {
    let p = CascadePolicy::default();
    assert!(p.allow_threshold > p.deny_threshold);
    // 0.93 is enough for a deny and not for an allow
    let r = |lean| Reading {
        lean,
        probability: 0.93,
        confidence: 0.93,
    };
    assert!(p.accepts(&r(Lean::Deny)).is_some());
    assert!(p.accepts(&r(Lean::Allow)).is_none());
}

#[test]
fn a_redact_is_always_escalated_by_default() {
    let s = FakeSecond::says(Verdict::Redact("clean text".into()));
    let f = FakeReader::says(Lean::Redact, 1.0, 1.0);
    let d = run(&f, &s, CascadePolicy::default());
    assert_eq!(d.verdict, Verdict::Redact("clean text".into()));
    assert_eq!(d.stage, Stage::Second);
    assert_eq!(*s.asked.borrow(), 1);
}

#[test]
fn a_redact_can_be_accepted_as_a_placeholder_when_the_policy_says_so() {
    let policy = CascadePolicy::new(Some(0.9), Some(0.98), Some(0.9)).unwrap();
    let s = FakeSecond::says(Verdict::Allow);
    let f = FakeReader::says(Lean::Redact, 0.95, 0.9);
    let d = run(&f, &s, policy);
    assert_eq!(d.verdict, Verdict::Redact(FIRST_STAGE_REDACTION.into()));
    assert_eq!(d.stage, Stage::First);
}

#[test]
fn no_reading_escalates() {
    let (f, s) = (FakeReader::nothing(), FakeSecond::says(Verdict::Allow));
    let d = run(&f, &s, CascadePolicy::default());
    assert_eq!(d.verdict, Verdict::Allow);
    assert_eq!(d.stage, Stage::Second);
}

#[test]
fn a_knob_set_to_none_never_accepts_that_outcome() {
    let policy = CascadePolicy::new(None, None, None).unwrap();
    let s = FakeSecond::says(Verdict::Defer);
    for lean in [Lean::Allow, Lean::Redact, Lean::Deny] {
        let f = FakeReader::says(lean, 1.0, 1.0);
        assert_eq!(run(&f, &s, policy).stage, Stage::Second);
    }
}

#[test]
fn a_deferring_second_judge_defers_the_cascade_and_never_falls_back_to_the_first() {
    // the first judge would have allowed it with 0.9 (not enough); the second defers
    let (f, s) = (
        FakeReader::says(Lean::Allow, 0.9, 0.9),
        FakeSecond::says(Verdict::Defer),
    );
    let d = run(&f, &s, CascadePolicy::default());
    assert_eq!(d.verdict, Verdict::Defer);
    assert_eq!(d.stage, Stage::Second);
    // and a first-stage deny that was not confident enough is not used either
    let f = FakeReader::says(Lean::Deny, 0.5, 0.0);
    assert_eq!(
        run(&f, &s, CascadePolicy::default()).verdict,
        Verdict::Defer
    );
}

#[test]
fn the_cascade_never_allows_what_the_second_judge_would_not_when_it_escalated() {
    for second in [
        Verdict::Defer,
        Verdict::Deny("x".into()),
        Verdict::Redact("y".into()),
    ] {
        for (lean, p, c) in [
            (Lean::Allow, 0.97, 0.97),
            (Lean::Allow, 0.2, 0.2),
            (Lean::Deny, 0.1, 0.0),
            (Lean::Redact, 1.0, 1.0),
        ] {
            let (f, s) = (
                FakeReader::says(lean, p, c),
                FakeSecond::says(second.clone()),
            );
            let d = run(&f, &s, CascadePolicy::default());
            assert_eq!(d.stage, Stage::Second);
            assert_ne!(d.verdict, Verdict::Allow, "{second:?} {lean:?} {p}");
        }
    }
}

#[test]
fn a_nan_reading_escalates() {
    let policy = CascadePolicy::default();
    for lean in [Lean::Allow, Lean::Deny, Lean::Redact] {
        let r = Reading {
            lean,
            probability: f64::NAN,
            confidence: f64::NAN,
        };
        assert_eq!(policy.accepts(&r), None);
    }
}

#[test]
fn thresholds_outside_zero_to_one_are_refused() {
    for bad in [-0.1, 1.1, f64::NAN, f64::INFINITY] {
        assert!(CascadePolicy::new(Some(bad), None, None).is_err());
        assert!(CascadePolicy::new(None, Some(bad), None).is_err());
        assert!(CascadePolicy::new(None, None, Some(bad)).is_err());
    }
    assert!(CascadePolicy::new(Some(0.0), Some(1.0), None).is_ok());
}

#[test]
fn it_is_a_judge_in_the_slot() {
    let (f, s) = (
        FakeReader::says(Lean::Deny, 0.99, 0.9),
        FakeSecond::says(Verdict::Allow),
    );
    let j: Box<dyn Judge> = Box::new(CascadeJudge::new(&f, &s, CascadePolicy::default()));
    assert!(matches!(j.judge(&item("public")), Verdict::Deny(_)));
}

// the gated first stage

struct Transport {
    sent: RefCell<Vec<String>>,
    reply: Result<Reading, String>,
}
impl ReadingTransport for &Transport {
    fn read(&self, text: &str) -> Result<Reading, String> {
        self.sent.borrow_mut().push(text.to_string());
        self.reply.clone()
    }
}

fn transport(reply: Result<Reading, String>) -> Transport {
    Transport {
        sent: RefCell::new(vec![]),
        reply,
    }
}

fn reading() -> Reading {
    Reading {
        lean: Lean::Deny,
        probability: 0.99,
        confidence: 0.9,
    }
}

fn on(max: Option<&str>) -> HostedConfig {
    HostedConfig {
        enabled: true,
        max_label: max.map(String::from),
    }
}

#[test]
fn the_first_stage_is_off_by_default_and_gated_like_every_hosted_reader() {
    let t = transport(Ok(reading()));
    let off = GatedReader::new(&t, LabelChain::default(), HostedConfig::default());
    assert_eq!(off.read(&item("public")), None);
    let g = GatedReader::new(&t, LabelChain::default(), on(Some("internal")));
    for label in ["internal", "private", "secret", "", "unknown", "PUBLIC"] {
        assert_eq!(g.read(&item(label)), None, "{label:?}");
    }
    assert!(t.sent.borrow().is_empty());
    assert_eq!(g.read(&item("public")), Some(reading()));
    assert_eq!(t.sent.borrow().len(), 1);
}

#[test]
fn a_failing_first_stage_is_no_reading() {
    let t = transport(Err("timeout".into()));
    let g = GatedReader::new(&t, LabelChain::default(), on(Some("internal")));
    assert_eq!(g.read(&item("public")), None);
}

#[test]
fn text_at_or_above_the_ceiling_goes_to_neither_stage_and_the_cascade_defers() {
    let t = transport(Ok(reading()));
    let hosted = transport(Ok(Reading {
        lean: Lean::Allow,
        probability: 1.0,
        confidence: 1.0,
    }));
    struct Second<'a>(&'a Transport);
    impl Judge for Second<'_> {
        fn judge(&self, item: &ReviewItem) -> Verdict {
            // a hosted second stage behind its own gate
            let j = super::super::HostedJudge::new(
                Via(self.0),
                LabelChain::default(),
                on(Some("internal")),
            );
            j.judge(item)
        }
    }
    struct Via<'a>(&'a Transport);
    impl super::super::HostedTransport for Via<'_> {
        fn screen(&self, text: &str) -> Result<Verdict, String> {
            self.0.sent.borrow_mut().push(text.to_string());
            Ok(Verdict::Allow)
        }
    }
    let first = GatedReader::new(&t, LabelChain::default(), on(Some("internal")));
    let c = CascadeJudge::new(first, Second(&hosted), CascadePolicy::default());
    for label in ["internal", "private", "secret"] {
        let d = c.decide(&item(label));
        assert_eq!(d.verdict, Verdict::Defer, "{label}");
    }
    assert!(t.sent.borrow().is_empty());
    assert!(hosted.sent.borrow().is_empty());
}
