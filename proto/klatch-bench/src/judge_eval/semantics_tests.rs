use super::*;
use crate::judge_eval::corpus::Truth;
use crate::judge_eval::metrics::{Called, Record, summarize};

fn rec(id: &str, truth: Truth, verdict: Called) -> Record {
    Record {
        id: id.into(),
        category: "c".into(),
        injection: false,
        truth,
        verdict,
        redaction_leaked: false,
        redaction_data_leaked: false,
        contested: false,
        raw: None,
        probability: None,
        stage: None,
        latency_ms: 0.0,
        tokens_in: None,
        tokens_out: None,
        cost_usd: None,
        error: None,
    }
}

/// A fixed mini corpus as the rules judge answers it: two clean items it does not match (Allow), a redact item and
/// a deny item it does not match (Allow), and a deny item it stops (Deny).
fn mini() -> Vec<Record> {
    vec![
        rec("a1", Truth::Allow, Called::Allow),
        rec("a2", Truth::Allow, Called::Allow),
        rec("r1", Truth::Redact, Called::Allow),
        rec("d1", Truth::Deny, Called::Allow),
        rec("d2", Truth::Deny, Called::Deny),
    ]
}

#[test]
fn the_three_outcomes_add_up_to_the_corpus_size_under_both_semantics() {
    for s in [Semantics::NotStopped, Semantics::Production] {
        let o = outcomes(&mini(), s);
        assert_eq!(o.n, 5);
        assert_eq!(o.blocked + o.deferred + o.allowed, o.n, "{s:?}");
        assert_eq!(
            o.should_not_cross_blocked + o.should_not_cross_deferred + o.should_not_cross_allowed,
            o.should_not_cross,
            "{s:?}"
        );
    }
}

#[test]
fn a_deferred_item_is_held_under_production_and_leaked_under_the_old_semantics() {
    let old = outcomes(&mini(), Semantics::NotStopped);
    let prod = outcomes(&mini(), Semantics::Production);
    assert_eq!((old.leaked, old.deferred), (2, 0));
    assert_eq!((prod.leaked, prod.deferred), (0, 4));
    assert_eq!(prod.should_not_cross_deferred, 2);
    assert_eq!(prod.allowed, 0);
}

#[test]
fn the_old_figure_is_unchanged_and_matches_the_summary() {
    let records = mini();
    let old = outcomes(&records, Semantics::NotStopped);
    let s = summarize(&records, 0.01);
    assert_eq!(old.leaked, s.leaks);
    assert_eq!(old.should_not_cross, 3);
    assert_eq!(old.leaked, 2);
    assert!((old.leak_rate() - 2.0 / 3.0).abs() < 1e-12);
}

#[test]
fn a_judge_that_really_allows_keeps_its_allow_under_production() {
    // only the rules judge is reread: the caller passes Production for it alone
    let o = outcomes(&mini(), Semantics::NotStopped);
    assert_eq!(o.allowed, 4);
}

#[test]
fn contested_items_are_left_out_as_in_the_summary() {
    let mut records = mini();
    records[2].contested = true;
    assert_eq!(outcomes(&records, Semantics::Production).n, 4);
}

#[test]
fn the_report_and_json_name_the_semantics() {
    let records = mini();
    let text = format_section(&records);
    assert!(text.contains("not stopped by the rules"), "{text}");
    assert!(
        text.contains("production (deferred items are held"),
        "{text}"
    );
    assert!(text.contains("reaches the reviewer"), "{text}");
    let j = to_json(&records);
    assert_eq!(
        j["not_stopped_by_rules"]["semantics"],
        "not-stopped-by-rules"
    );
    assert_eq!(j["production"]["semantics"], "production");
    assert_eq!(j["not_stopped_by_rules"]["leak_metric"], "not_stopped_rate");
    assert_eq!(
        j["production"]["leak_metric"],
        "would_cross_without_reviewer_rate"
    );
    assert_eq!(j["not_stopped_by_rules"]["leaked"], 2);
    assert_eq!(j["production"]["leaked"], 0);
    assert_eq!(j["production"]["deferred_to_reviewer"], 4);
}
