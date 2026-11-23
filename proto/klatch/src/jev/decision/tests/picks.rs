use super::*;

// ---- the judge ----

#[test]
fn a_confident_allow_redact_and_deny_map_to_verdicts() {
    let p = |c: &str| ans(c, &[("allow", 0.0), ("redact", 0.0), ("deny", 0.0)], 0.95);
    let mk = |c: &str, v: f64| {
        let mut a = p(c);
        a.probabilities.insert(c.into(), v);
        a
    };
    assert_eq!(judge_verdict(&mk("allow", 0.95), 0.8), Verdict::Allow);
    assert!(
        matches!(judge_verdict(&mk("redact", 0.95), 0.8), Verdict::Redact(t) if t == REDACTED_PLACEHOLDER)
    );
    assert!(matches!(
        judge_verdict(&mk("deny", 0.95), 0.8),
        Verdict::Deny(_)
    ));
    assert_eq!(judge_verdict(&mk("other", 0.95), 0.8), Verdict::Defer);
}

#[test]
fn what_changed_narrowing_verdicts_need_only_the_probability_but_allow_needs_both() {
    // three options, probability 0.85 for the pick, confidence 0.775: below a threshold of 0.8 as a confidence
    let mk = |c: &str| {
        let mut a = ans(
            c,
            &[("allow", 0.075), ("redact", 0.075), ("deny", 0.075)],
            0.775,
        );
        a.probabilities.insert(c.into(), 0.85);
        a
    };
    assert!(
        matches!(judge_verdict(&mk("deny"), 0.8), Verdict::Deny(_)),
        "a deny at 85 percent is acted on"
    );
    assert!(matches!(
        judge_verdict(&mk("redact"), 0.8),
        Verdict::Redact(_)
    ));
    assert_eq!(
        judge_verdict(&mk("allow"), 0.8),
        Verdict::Defer,
        "the one widening verdict also needs the confidence"
    );
    // below the probability threshold nothing is acted on
    let mut weak = mk("deny");
    weak.probabilities.insert("deny".into(), 0.79);
    assert_eq!(judge_verdict(&weak, 0.8), Verdict::Defer);
    let mut weak = mk("allow");
    weak.confidence = 0.95;
    weak.probabilities.insert("allow".into(), 0.79);
    assert_eq!(judge_verdict(&weak, 0.8), Verdict::Defer);
}

// ---- pick ----

#[test]
fn a_confident_answer_among_candidates_is_taken() {
    let a = ans(
        "topic-api",
        &[("topic-api", 0.95), ("topic-ui", 0.04), ("none", 0.01)],
        0.93,
    );
    let c = s(&["topic-api", "topic-ui"]);
    assert_eq!(pick(&c, None, &a, 0.9), Some("topic-api".into()));
    assert_eq!(
        pick(&c, Some("topic-ui"), &a, 0.9),
        Some("topic-api".into())
    );
}

#[test]
fn below_the_threshold_the_rules_decision_stands() {
    let a = ans(
        "topic-api",
        &[("topic-api", 0.6), ("topic-ui", 0.3), ("none", 0.1)],
        0.4,
    );
    let c = s(&["topic-api", "topic-ui"]);
    assert_eq!(pick(&c, Some("topic-ui"), &a, 0.9), Some("topic-ui".into()));
    assert_eq!(pick(&c, None, &a, 0.9), None);
}

#[test]
fn a_confident_none_narrows_to_nothing_on_the_probability_alone() {
    let a = ans(
        "none",
        &[("none", 0.85), ("topic-api", 0.075), ("topic-ui", 0.075)],
        0.775,
    );
    let c = s(&["topic-api", "topic-ui"]);
    assert_eq!(pick(&c, Some("topic-api"), &a, 0.8), None);
    assert_eq!(
        pick(&c, Some("topic-api"), &a, 0.9),
        Some("topic-api".into()),
        "below the threshold the rules stand"
    );
}

#[test]
fn a_name_outside_the_candidates_is_never_taken() {
    let a = ans("intruder", &[("intruder", 0.99), ("topic-api", 0.01)], 0.98);
    let c = s(&["topic-api"]);
    assert_eq!(
        pick(&c, Some("topic-api"), &a, 0.5),
        Some("topic-api".into())
    );
    assert_eq!(pick(&c, None, &a, 0.5), None);
}

#[test]
fn a_candidate_must_be_confident_on_both_numbers() {
    // probability 0.85 clears 0.8 but the confidence of 0.775 does not: a candidate is not taken on one number
    let a = ans(
        "topic-api",
        &[("topic-api", 0.85), ("topic-ui", 0.075), ("none", 0.075)],
        0.775,
    );
    let c = s(&["topic-api", "topic-ui"]);
    assert_eq!(pick(&c, Some("topic-ui"), &a, 0.8), Some("topic-ui".into()));
}

// ---- the screen in front of the pick ----

#[test]
fn a_clear_no_narrows_to_nothing_unsure_leaves_the_rules_and_a_clear_yes_asks_the_pick() {
    let a = ans("topic-api", &[("topic-api", 0.95), ("topic-ui", 0.05)], 0.9);
    let c = s(&["topic-api", "topic-ui"]);
    assert_eq!(pick_screened(&c, Some("topic-ui"), Band::No, &a, 0.9), None);
    assert_eq!(
        pick_screened(&c, Some("topic-ui"), Band::Unsure, &a, 0.9),
        Some("topic-ui".into())
    );
    assert_eq!(pick_screened(&c, None, Band::Unsure, &a, 0.9), None);
    assert_eq!(
        pick_screened(&c, Some("topic-ui"), Band::Yes, &a, 0.9),
        Some("topic-api".into())
    );
    // a yes never lets a name outside the candidates through either
    let out = ans("intruder", &[("intruder", 0.99), ("topic-api", 0.01)], 0.98);
    assert_eq!(
        pick_screened(&c, Some("topic-ui"), Band::Yes, &out, 0.5),
        Some("topic-ui".into())
    );
}
