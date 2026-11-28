//! Feeding items to a judge: each corpus item becomes the `ReviewItem` the gateway's drain would hand a judge
//! (#48): a crossing from the item's label down to its `to` label, timed by the clock the caller passes.

use crate::judge_eval::corpus::Item;
use crate::judge_eval::judges::EvalJudge;
use crate::judge_eval::metrics::{Called, Record};
use klatch::domain::{ReviewItem, Verdict};

pub fn review_item(n: usize, item: &Item) -> ReviewItem {
    ReviewItem {
        id: n as i64,
        room: "judge-eval".into(),
        seq: n as i64,
        to_label: item.to.clone(),
        from_label: item.label.clone(),
        reason: "crossing".into(),
        text: item.text.clone(),
        // judge-eval items are not crossing requests: no target room, and no judge prompt uses it
        to_room: String::new(),
    }
}

/// Judge every item once, in order. `clock` returns milliseconds; `progress` is told (done, total) after each item.
pub fn run_items(
    judge: &dyn EvalJudge,
    items: &[&Item],
    clock: &mut dyn FnMut() -> f64,
    progress: &mut dyn FnMut(usize, usize),
) -> Vec<Record> {
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let review = review_item(i + 1, item);
        let started = clock();
        let judged = judge.evaluate(&review);
        let latency_ms = clock() - started;
        let (verdict, redaction_leaked, redaction_data_leaked) = match &judged.verdict {
            Verdict::Allow => (Called::Allow, false, false),
            Verdict::Redact(text) => (
                Called::Redact,
                item.spans.iter().any(|s| text.contains(s.as_str())),
                item.data_spans().any(|s| text.contains(s.as_str())),
            ),
            Verdict::Deny(_) => (Called::Deny, false, false),
            Verdict::Defer => (Called::Defer, false, false),
        };
        out.push(Record {
            id: item.id.clone(),
            category: item.category.clone(),
            injection: item.is_injection(),
            truth: item.verdict,
            verdict,
            redaction_leaked,
            redaction_data_leaked,
            contested: item.contested,
            raw: judged.raw,
            probability: judged.probability,
            stage: judged.stage,
            latency_ms,
            tokens_in: judged.tokens_in,
            tokens_out: judged.tokens_out,
            cost_usd: judged.cost_usd,
            error: judged.error,
        });
        progress(i + 1, items.len());
    }
    out
}
