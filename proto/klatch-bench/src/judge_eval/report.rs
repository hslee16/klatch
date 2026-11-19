//! The printed report and the results JSON. Neither ever holds an item's text: ids, verdicts and numbers only.

use crate::judge_eval::corpus::Truth;
use crate::judge_eval::interval::{MIN_N, rate_with_interval};
use crate::judge_eval::metrics::{Called, PerVerdict, Record, Summary};
use serde_json::{Value, json};
use std::fmt::Write;

fn pct(x: f64) -> String {
    format!("{:.1}%", x * 100.0)
}

fn opt_pct(x: Option<f64>) -> String {
    x.map_or_else(|| "-".to_string(), pct)
}

fn opt<T: std::fmt::Display>(x: Option<T>) -> String {
    x.map_or_else(|| "-".to_string(), |v| v.to_string())
}

/// The report as printed after a run.
pub fn format(s: &Summary, judge: &str, split: &str) -> String {
    let mut o = String::new();
    let blocked: usize =
        s.confusion[1].iter().sum::<usize>() + s.confusion[2].iter().sum::<usize>();
    let clean: usize = s.confusion[0].iter().sum();
    let _ = writeln!(o, "judge-eval: judge {judge}, split {split}, {} items", s.n);
    let correct = (s.accuracy * s.n as f64).round() as usize;
    let decided = s.n - s.defers;
    let mut small: Vec<String> = vec![];
    let mut note = |name: &str, n: usize| {
        if n < MIN_N {
            small.push(format!("{name} n={n}"));
        }
    };
    note("leak rate", blocked);
    note("over-blocking rate", clean);
    note("deferral rate", s.n);
    note("accuracy on decided items", decided);
    for t in Truth::ALL {
        note(
            &format!("{} recall", t.word()),
            s.confusion[t.index()].iter().sum(),
        );
    }
    for r in s.by_category.iter().chain(std::iter::once(&s.injection)) {
        note(&format!("category {}", r.name), r.n);
    }
    if !small.is_empty() {
        let _ = writeln!(
            o,
            "warning: n is below {MIN_N} for: {}; those intervals are wide, read the rates with care",
            small.join(", ")
        );
    }
    let _ = writeln!(o, "intervals are Wilson 95 percent confidence intervals");
    let _ = writeln!(o, "\nconfusion (rows: ground truth, columns: verdict)");
    let _ = writeln!(
        o,
        "  {:<8}{:>8}{:>8}{:>8}{:>8}",
        "", "allow", "redact", "deny", "defer"
    );
    for t in Truth::ALL {
        let row = s.confusion[t.index()];
        let _ = writeln!(
            o,
            "  {:<8}{:>8}{:>8}{:>8}{:>8}",
            t.word(),
            row[0],
            row[1],
            row[2],
            row[3]
        );
    }
    let _ = writeln!(
        o,
        "\nleaks (critical: a redact or deny item let across): {} of {blocked} = {} ({} were redactions that kept a span)",
        s.leaks,
        rate_with_interval(s.leaks, blocked),
        s.redaction_leaks
    );
    let _ = writeln!(
        o,
        "over-blocking (a clean item redacted or denied): {} of {clean} = {}",
        s.overblocks,
        rate_with_interval(s.overblocks, clean)
    );
    let _ = writeln!(
        o,
        "softened (a deny item only redacted, nothing crossed): {}",
        s.softened
    );
    let _ = writeln!(
        o,
        "deferred (no decision, neither right nor wrong): {} of {} = {}; failed calls: {}",
        s.defers,
        s.n,
        rate_with_interval(s.defers, s.n),
        s.errors
    );
    let _ = writeln!(
        o,
        "accuracy: {} of all items, {} of decided items",
        pct(s.accuracy),
        rate_with_interval(correct.min(decided), decided)
    );
    let _ = writeln!(
        o,
        "\nprecision and recall (recall counts a deferred item as missed)"
    );
    for p in &s.per_verdict {
        let _ = writeln!(
            o,
            "  {:<8} precision {:>7}  recall {}",
            p.verdict.word(),
            opt_pct(p.precision),
            rate_with_interval(
                recall_hits(p, s.confusion[p.verdict.index()].iter().sum()),
                s.confusion[p.verdict.index()].iter().sum()
            )
        );
    }
    let ms = |x: Option<f64>| x.map_or_else(|| "-".to_string(), |v| format!("{v:.0} ms"));
    let _ = writeln!(
        o,
        "\nlatency: p50 {}, p99 {}",
        ms(s.latency_p50_ms),
        ms(s.latency_p99_ms)
    );
    let _ = writeln!(
        o,
        "tokens: in {}, out {}; cost: {}",
        opt(s.tokens_in),
        opt(s.tokens_out),
        s.cost_usd
            .map_or_else(|| "-".to_string(), |c| format!("${c:.4}"))
    );
    match &s.calibration {
        None => {
            let _ = writeln!(o, "\ncalibration: the judge reports no confidence");
        }
        Some(c) => {
            let _ = writeln!(
                o,
                "\ncalibration of the judge's own pick (ECE {:.3})",
                c.ece
            );
            let _ = writeln!(
                o,
                "  {:<12}{:>6}{:>12}{:>16}",
                "confidence", "n", "accuracy", "mean confidence"
            );
            for b in &c.buckets {
                let _ = writeln!(
                    o,
                    "  {:<12}{:>6}{:>12}{:>16}",
                    format!("{:.1}-{:.1}", b.lo, b.hi),
                    b.n,
                    pct(b.accuracy),
                    format!("{:.3}", b.mean_confidence)
                );
            }
        }
    }
    if !s.thresholds.is_empty() {
        let _ = writeln!(
            o,
            "\nconfidence threshold table (a pick below the threshold becomes a defer)"
        );
        let _ = writeln!(
            o,
            "  {:<10}{:>8}{:>10}{:>12}{:>16}{:>10}",
            "threshold", "leaks", "leak rate", "over-blocks", "over-block rate", "deferred"
        );
        for r in &s.thresholds {
            let _ = writeln!(
                o,
                "  {:<10}{:>8}{:>10}{:>12}{:>16}{:>10}",
                format!("{:.2}", r.threshold),
                r.leaks,
                pct(r.leak_rate),
                r.overblocks,
                pct(r.overblock_rate),
                r.deferred
            );
        }
        let _ = writeln!(
            o,
            "  lowest threshold with a leak rate of at most {}: {} (a reading of this table, not a decision)",
            pct(s.target_leak),
            s.lowest_threshold
                .map_or_else(|| "none in the table".to_string(), |t| format!("{t:.2}"))
        );
    }
    let _ = writeln!(o, "\nby category");
    let _ = writeln!(
        o,
        "  {:<22}{:>5}{:>9}{:>7}{:>13}{:>8}  correct rate (95% CI)",
        "category", "n", "correct", "leaks", "over-blocks", "defers"
    );
    for r in s.by_category.iter().chain(std::iter::once(&s.injection)) {
        let _ = writeln!(
            o,
            "  {:<22}{:>5}{:>9}{:>7}{:>13}{:>8}  {}",
            r.name,
            r.n,
            r.correct,
            r.leaks,
            r.overblocks,
            r.defers,
            rate_with_interval(r.correct, r.n)
        );
    }
    o
}

/// The recall numerator (hits are not the diagonal of the confusion: a redaction that kept a span is a leak).
fn recall_hits(p: &PerVerdict, n: usize) -> usize {
    p.recall.map_or(0, |r| (r * n as f64).round() as usize)
}

fn called(c: Called) -> &'static str {
    c.word()
}

/// The results file: the summary and one row per item (id, truth, verdict and what the judge reported).
pub fn to_json(s: &Summary, records: &[Record], judge: &str, split: &str) -> Value {
    let per_verdict: Vec<Value> = s
        .per_verdict
        .iter()
        .map(|p| json!({"verdict": p.verdict.word(), "precision": p.precision, "recall": p.recall}))
        .collect();
    let calibration = s.calibration.as_ref().map(|c| {
        json!({
            "ece": c.ece,
            "buckets": c.buckets.iter().map(|b| json!({
                "lo": b.lo, "hi": b.hi, "n": b.n, "accuracy": b.accuracy, "mean_confidence": b.mean_confidence
            })).collect::<Vec<_>>()
        })
    });
    let thresholds: Vec<Value> = s
        .thresholds
        .iter()
        .map(|r| {
            json!({"threshold": r.threshold, "leaks": r.leaks, "leak_rate": r.leak_rate,
                   "overblocks": r.overblocks, "overblock_rate": r.overblock_rate, "deferred": r.deferred})
        })
        .collect();
    let categories: Vec<Value> = s
        .by_category
        .iter()
        .chain(std::iter::once(&s.injection))
        .map(|r| {
            json!({"category": r.name, "n": r.n, "correct": r.correct, "leaks": r.leaks,
                   "overblocks": r.overblocks, "defers": r.defers})
        })
        .collect();
    let items: Vec<Value> = records
        .iter()
        .map(|r| {
            json!({
                "id": r.id, "category": r.category, "truth": r.truth.word(), "verdict": called(r.verdict),
                "leak": r.is_leak(), "overblock": r.is_overblock(),
                "pick": r.raw.map(|(t, _)| t.word()), "confidence": r.raw.map(|(_, c)| c),
                "latency_ms": r.latency_ms, "tokens_in": r.tokens_in, "tokens_out": r.tokens_out,
                "cost_usd": r.cost_usd, "error": r.error
            })
        })
        .collect();
    json!({
        "judge": judge,
        "split": split,
        "summary": {
            "n": s.n,
            "confusion": {"rows": ["allow", "redact", "deny"], "columns": ["allow", "redact", "deny", "defer"], "counts": s.confusion},
            "leaks": s.leaks, "leak_rate": s.leak_rate, "redaction_leaks": s.redaction_leaks,
            "overblocks": s.overblocks, "overblock_rate": s.overblock_rate, "softened": s.softened,
            "defers": s.defers, "defer_rate": s.defer_rate, "errors": s.errors,
            "accuracy": s.accuracy, "accuracy_decided": s.accuracy_decided,
            "per_verdict": per_verdict,
            "latency_p50_ms": s.latency_p50_ms, "latency_p99_ms": s.latency_p99_ms,
            "tokens_in": s.tokens_in, "tokens_out": s.tokens_out, "cost_usd": s.cost_usd,
            "calibration": calibration, "thresholds": thresholds,
            "target_leak": s.target_leak, "lowest_threshold": s.lowest_threshold,
            "by_category": categories
        },
        "items": items
    })
}
