use super::*;
use klatch::domain::{ReviewItem, Verdict};

fn review(text: &str, to: &str) -> ReviewItem {
    ReviewItem {
        id: 1,
        room: "judge-eval".into(),
        seq: 1,
        to_label: to.into(),
        from_label: "private".into(),
        reason: "crossing".into(),
        text: text.into(),
    }
}

fn rules() -> RulesJudge {
    RulesJudge::new(
        parse_scrub("# private terms\nhost = orca-build-07\nproject=Project Larkspur\n\n").unwrap(),
    )
}

#[test]
fn the_scrub_list_is_name_equals_term_with_comments_and_blank_lines() {
    let t = parse_scrub("# c\n a = b c \n\nx=y\n").unwrap();
    assert_eq!(
        t,
        vec![
            ("a".to_string(), "b c".to_string()),
            ("x".into(), "y".into())
        ]
    );
    assert!(parse_scrub("no equals sign").is_err());
    assert!(parse_scrub("name = ").is_err());
    assert!(parse_scrub("bad name! = term").is_err());
}

#[test]
fn the_rules_deny_a_credential_the_engine_recognises() {
    let key = crate::judge_eval::corpus::expand("r", "{{fake:github}}").unwrap();
    let v = rules().judge(&review(&format!("use {key} for ci"), "public"));
    assert!(
        matches!(v, Verdict::Deny(ref why) if why.contains("GitHub")),
        "{v:?}"
    );
}

#[test]
fn the_rules_deny_a_listed_term_in_any_case_and_name_only_the_rule() {
    let v = rules().judge(&review("the ORCA-BUILD-07 box is down", "public"));
    match v {
        Verdict::Deny(why) => {
            assert!(why.contains("scrub:host"), "{why}");
            assert!(!why.to_lowercase().contains("orca"), "{why}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_rules_allow_everything_else_and_never_redact() {
    for text in [
        "tests pass on main",
        "orca build 07 is a near miss",
        "AKIA is the prefix of an AWS key",
        "commit 3f9c1d2a8b7e6f5d4c3b2a1908f7e6d5c4b3a291",
    ] {
        assert_eq!(
            rules().judge(&review(text, "public")),
            Verdict::Allow,
            "{text}"
        );
    }
}

#[test]
fn a_destination_at_or_above_the_agent_cap_is_held_to_the_credential_rule_only() {
    // as in the gateway: the scrubber covers rooms below the cap (default private)
    let r = rules();
    assert_eq!(
        r.judge(&review("orca-build-07 is down", "private")),
        Verdict::Allow
    );
    let key = crate::judge_eval::corpus::expand("r", "{{fake:aws}}").unwrap();
    assert!(matches!(
        r.judge(&review(&key, "private")),
        Verdict::Deny(_)
    ));
}

#[test]
fn the_production_null_judge_goes_through_the_seam_unchanged() {
    let j = Seam::new("null", klatch::domain::NullJudge);
    assert_eq!(j.name(), "null");
    let out = j.evaluate(&review("x", "public"));
    assert_eq!(out.verdict, Verdict::Defer);
    assert!(out.raw.is_none() && out.error.is_none() && out.cost_usd.is_none());
}
