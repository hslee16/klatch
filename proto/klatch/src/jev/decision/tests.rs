use super::*;
use crate::domain::Verdict;
use crate::jev::answer::{ChoiceAnswer, ScoreAnswer};
use std::collections::BTreeMap;

fn ans(choice: &str, probs: &[(&str, f64)], conf: f64) -> ChoiceAnswer {
    ChoiceAnswer {
        choice: choice.into(),
        probabilities: probs
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect::<BTreeMap<_, _>>(),
        confidence: conf,
    }
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} vs {b}");
}

// ---- the documented formulas ----

#[test]
fn choice_confidence_is_the_documented_formula() {
    near(choice_confidence(&[0.6, 0.3, 0.1]), 0.4);
    near(choice_confidence(&[0.6, 0.2, 0.2]), 0.4); // only the top probability counts
    near(choice_confidence(&[0.75, 0.25]), 0.5);
    near(choice_confidence(&[1.0, 0.0, 0.0]), 1.0);
    near(choice_confidence(&[1.0 / 3.0; 3]), 0.0);
    near(choice_confidence(&[1.0]), 1.0);
}

#[test]
fn the_table_of_confidence_against_top_probability_holds() {
    // confidence.md: confidence 0.5 is a top probability of 0.75, 0.67 and 0.55 for 2, 3 and 10 options
    near(choice_confidence(&[0.75, 0.25]), 0.5);
    assert!((choice_confidence(&[0.67, 0.165, 0.165]) - 0.505).abs() < 0.01);
    let ten: Vec<f64> = std::iter::once(0.55)
        .chain(std::iter::repeat_n(0.45 / 9.0, 9))
        .collect();
    near(choice_confidence(&ten), 0.5);
}

#[test]
fn a_noul_has_no_confidence_so_the_distance_from_one_half_stands_in() {
    near(noul_confidence(0.5), 0.0);
    near(noul_confidence(0.0), 1.0);
    near(noul_confidence(1.0), 1.0);
    near(noul_confidence(0.85), 0.7);
}

#[test]
fn score_confidence_counts_the_distance_between_levels() {
    // confidence.md: (0, 0.5, 0.5) is 0.25 and (0.5, 0, 0.5) is 0 for three levels; (0, 0.57, 0.43) is about 0.35
    near(score_confidence(&[0.0, 0.5, 0.5]), 0.25);
    near(score_confidence(&[0.5, 0.0, 0.5]), 0.0);
    assert!((score_confidence(&[0.0, 0.57, 0.43]) - 0.355).abs() < 0.01);
    near(score_confidence(&[0.0, 0.0, 1.0]), 1.0);
}

#[test]
fn the_top_to_second_ratio_tells_a_close_runner_up() {
    let a = ans("a", &[("a", 0.6), ("b", 0.3), ("c", 0.1)], 0.4);
    near(top_to_second_ratio(&a).unwrap(), 2.0);
    let sure = ans("a", &[("a", 1.0), ("b", 0.0)], 1.0);
    assert!(top_to_second_ratio(&sure).unwrap().is_infinite());
    let lone = ans("a", &[("a", 1.0)], 1.0);
    assert_eq!(top_to_second_ratio(&lone), None);
}

fn severity() -> ScoreAnswer {
    // the confidence-routing.md example: most of the mass on the top level, some on a low one
    ScoreAnswer {
        score: 3.19,
        confidence: 0.34,
        probabilities: vec![0.01, 0.14, 0.07, 0.19, 0.59],
        legend: vec![],
    }
}

#[test]
fn a_score_is_settled_only_at_or_above_the_confidence_floor_and_the_mass_of_levels_adds_up() {
    let a = severity();
    assert!(
        !a.is_settled(0.5),
        "confidence 0.34 is below the documented 0.5 floor"
    );
    assert!(a.is_settled(0.34));
    near(a.level_mass(3..=4), 0.78);
    near(a.level_mass(0..=0), 0.01);
    near(a.level_mass(4..=9), 0.59); // levels past the scale add nothing
    near(a.level_mass(5..=9), 0.0);
}

// ---- noul screen ----

#[test]
fn a_noul_is_a_clear_yes_a_clear_no_or_unsure() {
    let t = NoulThresholds::default();
    assert_eq!((t.yes_above(), t.no_below()), (0.7, 0.3));
    assert_eq!(t.band(0.98), Band::Yes);
    assert_eq!(t.band(0.71), Band::Yes);
    assert_eq!(t.band(0.7), Band::Unsure, "above 0.7, not at it");
    assert_eq!(t.band(0.44), Band::Unsure);
    assert_eq!(t.band(0.3), Band::Unsure, "below 0.3, not at it");
    assert_eq!(t.band(0.29), Band::No);
    assert_eq!(t.band(0.02), Band::No);
    assert_eq!(t.band(f64::NAN), Band::Unsure, "no number is not a yes");
}

#[test]
fn noul_thresholds_are_validated() {
    assert!(NoulThresholds::new(0.9, 0.1).is_ok());
    assert!(
        NoulThresholds::new(0.5, 0.5).is_err(),
        "the unsure band cannot vanish"
    );
    assert!(NoulThresholds::new(0.3, 0.7).is_err());
    assert!(NoulThresholds::new(1.1, 0.1).is_err());
    assert!(NoulThresholds::new(0.9, -0.1).is_err());
    assert!(NoulThresholds::new(f64::NAN, 0.1).is_err());
    assert!(NoulThresholds::new(0.9, f64::NAN).is_err());
}

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
