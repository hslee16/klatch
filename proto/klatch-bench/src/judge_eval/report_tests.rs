//! Golden tests of the printed report and the results JSON (#1451): the output of the runner is read by
//! `experiments/coordbench/judge_measure.py`, so its bytes are pinned over two fixed fixtures. To regenerate a
//! golden after a deliberate change, run the tests with `KLATCH_GOLDEN_UPDATE=1`.

use super::*;
use crate::judge_eval::corpus::Truth;
use crate::judge_eval::metrics::{Called, Record, summarize};
use std::path::PathBuf;

fn rec(id: &str, category: &str, truth: Truth, verdict: Called) -> Record {
    Record {
        id: id.into(),
        category: category.into(),
        injection: category.starts_with("injection"),
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

/// Plain: no confidence, no stages, no contested items, no cost, few items.
fn plain() -> Vec<Record> {
    vec![
        rec("p1", "everyday-chat", Truth::Allow, Called::Allow),
        rec("p2", "everyday-chat", Truth::Allow, Called::Redact),
        rec("p3", "personal-data", Truth::Redact, Called::Allow),
        rec("p4", "personal-data", Truth::Redact, Called::Redact),
        rec("p5", "secret-credential", Truth::Deny, Called::Deny),
        rec("p6", "injection-hidden", Truth::Deny, Called::Defer),
    ]
}

/// Rich: confidence, stages, a contested item, tokens, cost, errors, enough items to clear the small-n warning.
fn rich() -> Vec<Record> {
    let truths = [Truth::Allow, Truth::Allow, Truth::Redact, Truth::Deny];
    let calls = [
        Called::Allow,
        Called::Redact,
        Called::Deny,
        Called::Defer,
        Called::Allow,
    ];
    let cats = [
        "everyday-chat",
        "personal-data",
        "secret-credential",
        "injection-hidden",
    ];
    let mut out = vec![];
    for i in 0..48usize {
        let truth = truths[i % 4];
        let mut r = rec(
            &format!("r{i:02}"),
            cats[(i / 2) % 4],
            truth,
            calls[(i * 3 + i / 5) % 5],
        );
        r.redaction_leaked = r.verdict == Called::Redact && i % 3 == 0;
        r.redaction_data_leaked = r.redaction_leaked && i % 2 == 0;
        r.contested = i % 17 == 16;
        if r.verdict != Called::Defer {
            r.raw = Some((truths[(i + 1) % 4], ((i * 37) % 100) as f64 / 100.0));
        }
        r.probability = (i % 3 == 0).then(|| (i % 10) as f64 / 10.0);
        r.stage = Some(if i % 4 == 0 { "second" } else { "first" }.to_string());
        r.latency_ms = 10.0 + (i * 7 % 23) as f64 * 13.0;
        r.tokens_in = Some(100 + i as u64);
        r.tokens_out = Some(10 + (i % 5) as u64);
        r.cost_usd = Some(0.001 * (i + 1) as f64);
        r.error = (i % 11 == 10).then(|| "boom".to_string());
        out.push(r);
    }
    out
}

fn golden(name: &str, got: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/judge_eval/golden")
        .join(name);
    if std::env::var_os("KLATCH_GOLDEN_UPDATE").is_some() {
        std::fs::write(&path, got).unwrap();
    }
    let want = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(got, want, "golden {name} differs");
}

#[test]
fn the_plain_report_is_pinned() {
    let records = plain();
    let s = summarize(&records, 0.01);
    golden("report_plain.txt", &format(&s, "plain-judge", "dev"));
    let json = serde_json::to_string_pretty(&to_json(&s, &records, "plain-judge", "dev")).unwrap();
    golden("report_plain.json", &json);
}

#[test]
fn the_rich_report_is_pinned() {
    let records = rich();
    let s = summarize(&records, 0.05);
    golden("report_rich.txt", &format(&s, "cascade:a,b", "test"));
    let json = serde_json::to_string_pretty(&to_json(&s, &records, "cascade:a,b", "test")).unwrap();
    golden("report_rich.json", &json);
}
