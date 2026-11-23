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
    header(&mut o, s, judge, split);
    small_n_warning(&mut o, s, blocked, clean);
    confusion_table(&mut o, s);
    headline_rates(&mut o, s, blocked, clean);
    precision_recall(&mut o, s);
    escalation(&mut o, s);
    cost_lines(&mut o, s);
    calibration_section(&mut o, s);
    threshold_section(&mut o, s);
    category_section(&mut o, s);
    contested_section(&mut o, s);
    o
}

fn header(o: &mut String, s: &Summary, judge: &str, split: &str) {
    let held = if s.contested.is_empty() {
        String::new()
    } else {
        format!(
            "; {} contested excluded from every figure below",
            s.contested.len()
        )
    };
    let _ = writeln!(
        o,
        "judge-eval: judge {judge}, split {split}, {} items{held}",
        s.n
    );
}

/// The warning line naming every figure whose n is below `MIN_N`, then the note on the intervals.
fn small_n_warning(o: &mut String, s: &Summary, blocked: usize, clean: usize) {
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
}

fn confusion_table(o: &mut String, s: &Summary) {
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
}

/// Leaks, data leaks, over-blocking, softened, deferred and accuracy.
fn headline_rates(o: &mut String, s: &Summary, blocked: usize, clean: usize) {
    let correct = (s.accuracy * s.n as f64).round() as usize;
    let decided = s.n - s.defers;
    let _ = writeln!(
        o,
        "\nleaks (critical: a redact or deny item let across): {} of {blocked} = {} ({} were redactions that kept a span)",
        s.leaks,
        rate_with_interval(s.leaks, blocked),
        s.redaction_leaks
    );
    let _ = writeln!(
        o,
        "data leaks (a leak only when a non-name span is left): {} of {blocked} = {} (a redaction that removes the data but keeps a person's name is a strict leak above, not one here)",
        s.data_leaks,
        rate_with_interval(s.data_leaks, blocked)
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
}

fn precision_recall(o: &mut String, s: &Summary) {
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
}

fn escalation(o: &mut String, s: &Summary) {
    if s.staged == 0 {
        return;
    }
    let _ = writeln!(
        o,
        "\nescalated to the second judge: {} of {} = {} (the rest were decided by the first judge; \
latency and cost below are end to end, summed over the stages actually called)",
        s.escalated,
        s.staged,
        rate_with_interval(s.escalated, s.staged)
    );
}

/// Latency, tokens and cost.
fn cost_lines(o: &mut String, s: &Summary) {
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
}

fn calibration_section(o: &mut String, s: &Summary) {
    let Some(c) = &s.calibration else {
        let _ = writeln!(o, "\ncalibration: the judge reports no confidence");
        return;
    };
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

fn threshold_section(o: &mut String, s: &Summary) {
    if s.thresholds.is_empty() {
        return;
    }
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

fn category_section(o: &mut String, s: &Summary) {
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
}

fn contested_section(o: &mut String, s: &Summary) {
    if s.contested.is_empty() {
        return;
    }
    let _ = writeln!(
        o,
        "\ncontested items (scored apart: judged, but in no figure above; the label is open to dispute)"
    );
    let _ = writeln!(o, "  {:<12}{:>8}{:>10}  category", "id", "label", "verdict");
    for r in &s.contested {
        let _ = writeln!(
            o,
            "  {:<12}{:>8}{:>10}  {}",
            r.id,
            r.truth.word(),
            r.verdict.word(),
            r.category
        );
    }
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
    json!({
        "judge": judge,
        "split": split,
        "summary": summary_json(s),
        "items": records.iter().map(item_json).collect::<Vec<_>>()
    })
}

fn item_json(r: &Record) -> Value {
    json!({
        "id": r.id, "category": r.category, "truth": r.truth.word(), "verdict": called(r.verdict),
        "leak": r.is_leak(), "data_leak": r.is_data_leak(), "overblock": r.is_overblock(),
        "contested": r.contested,
        "stage": r.stage, "probability": r.probability,
        "pick": r.raw.map(|(t, _)| t.word()), "confidence": r.raw.map(|(_, c)| c),
        "latency_ms": r.latency_ms, "tokens_in": r.tokens_in, "tokens_out": r.tokens_out,
        "cost_usd": r.cost_usd, "error": r.error
    })
}

fn per_verdict_json(s: &Summary) -> Vec<Value> {
    s.per_verdict
        .iter()
        .map(|p| json!({"verdict": p.verdict.word(), "precision": p.precision, "recall": p.recall}))
        .collect()
}

fn calibration_json(s: &Summary) -> Option<Value> {
    s.calibration.as_ref().map(|c| {
        json!({
            "ece": c.ece,
            "buckets": c.buckets.iter().map(|b| json!({
                "lo": b.lo, "hi": b.hi, "n": b.n, "accuracy": b.accuracy, "mean_confidence": b.mean_confidence
            })).collect::<Vec<_>>()
        })
    })
}

fn thresholds_json(s: &Summary) -> Vec<Value> {
    s.thresholds
        .iter()
        .map(|r| {
            json!({"threshold": r.threshold, "leaks": r.leaks, "leak_rate": r.leak_rate,
                   "overblocks": r.overblocks, "overblock_rate": r.overblock_rate, "deferred": r.deferred})
        })
        .collect()
}

fn categories_json(s: &Summary) -> Vec<Value> {
    s.by_category
        .iter()
        .chain(std::iter::once(&s.injection))
        .map(|r| {
            json!({"category": r.name, "n": r.n, "correct": r.correct, "leaks": r.leaks,
                   "overblocks": r.overblocks, "defers": r.defers})
        })
        .collect()
}

fn contested_json(s: &Summary) -> Vec<Value> {
    s.contested
        .iter()
        .map(|c| {
            json!({
                "id": c.id, "category": c.category, "label": c.truth.word(), "verdict": c.verdict.word()
            })
        })
        .collect()
}

fn summary_json(s: &Summary) -> Value {
    json!({
        "n": s.n,
        "confusion": {"rows": ["allow", "redact", "deny"], "columns": ["allow", "redact", "deny", "defer"], "counts": s.confusion},
        "leaks": s.leaks, "leak_rate": s.leak_rate, "redaction_leaks": s.redaction_leaks,
        "data_leaks": s.data_leaks, "data_leak_rate": s.data_leak_rate,
        "contested": contested_json(s),
        "overblocks": s.overblocks, "overblock_rate": s.overblock_rate, "softened": s.softened,
        "defers": s.defers, "defer_rate": s.defer_rate, "errors": s.errors,
        "staged": s.staged, "escalated": s.escalated,
        "escalation_rate": (s.staged > 0).then(|| s.escalated as f64 / s.staged as f64),
        "accuracy": s.accuracy, "accuracy_decided": s.accuracy_decided,
        "per_verdict": per_verdict_json(s),
        "latency_p50_ms": s.latency_p50_ms, "latency_p99_ms": s.latency_p99_ms,
        "tokens_in": s.tokens_in, "tokens_out": s.tokens_out, "cost_usd": s.cost_usd,
        "calibration": calibration_json(s), "thresholds": thresholds_json(s),
        "target_leak": s.target_leak, "lowest_threshold": s.lowest_threshold,
        "by_category": categories_json(s)
    })
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
