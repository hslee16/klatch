use super::*;
use crate::judge_eval::corpus::{Item, Split, Truth};
use crate::judge_eval::judges::{EvalJudge, Judged};
use crate::judge_eval::run::run_items;
use klatch::domain::{ReviewItem, Verdict};
use std::cell::Cell;
use std::collections::HashMap;

fn item(id: &str, category: &str, truth: Truth) -> Item {
    let spans = if truth == Truth::Allow {
        vec![]
    } else {
        vec![format!("SPAN-{id}")]
    };
    Item {
        id: id.into(),
        split: Split::Dev,
        category: category.into(),
        label: "private".into(),
        to: "public".into(),
        verdict: truth,
        raw_text: format!("text of {id} SPAN-{id}"),
        text: format!("text of {id} SPAN-{id}"),
        rationale: "r".into(),
        spans,
        origin: None,
    }
}

/// Ten items: six allow, two redact, two deny; two of them injection.
fn corpus() -> Vec<Item> {
    vec![
        item("a1", "everyday-chat", Truth::Allow),
        item("a2", "everyday-chat", Truth::Allow),
        item("a3", "everyday-chat", Truth::Allow),
        item("a4", "code-log-clean", Truth::Allow),
        item("a5", "code-log-clean", Truth::Allow),
        item("a6", "injection-lookalike", Truth::Allow),
        item("r1", "personal-data", Truth::Redact),
        item("r2", "personal-data", Truth::Redact),
        item("d1", "secret-credential", Truth::Deny),
        item("d2", "injection-hidden", Truth::Deny),
    ]
}

/// Knows the answer: looks the item up by its text.
struct Perfect(HashMap<String, Truth>);
impl Perfect {
    fn of(items: &[Item]) -> Self {
        Perfect(items.iter().map(|i| (i.text.clone(), i.verdict)).collect())
    }
}
impl EvalJudge for Perfect {
    fn name(&self) -> String {
        "perfect".into()
    }
    fn evaluate(&self, item: &ReviewItem) -> Judged {
        let v = match self.0[&item.text] {
            Truth::Allow => Verdict::Allow,
            Truth::Redact => Verdict::Redact("[gone]".into()),
            Truth::Deny => Verdict::Deny("no".into()),
        };
        Judged::of(v)
    }
}

struct Always(Verdict);
impl EvalJudge for Always {
    fn name(&self) -> String {
        format!("always-{}", self.0.word())
    }
    fn evaluate(&self, _: &ReviewItem) -> Judged {
        Judged::of(self.0.clone())
    }
}

/// A fixed-seed random judge: a pick and a confidence from a xorshift stream.
struct Random(Cell<u64>);
impl Random {
    fn next(&self) -> u64 {
        let mut x = self.0.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0.set(x);
        x
    }
}
impl EvalJudge for Random {
    fn name(&self) -> String {
        "random".into()
    }
    fn evaluate(&self, _: &ReviewItem) -> Judged {
        let (v, t) = match self.next() % 4 {
            0 => (Verdict::Allow, Truth::Allow),
            1 => (Verdict::Redact("x".into()), Truth::Redact),
            2 => (Verdict::Deny("x".into()), Truth::Deny),
            _ => return Judged::of(Verdict::Defer),
        };
        let conf = (self.next() % 1000) as f64 / 1000.0;
        Judged {
            raw: Some((t, conf)),
            ..Judged::of(v)
        }
    }
}

fn clock() -> impl FnMut() -> f64 {
    let mut t = 0.0;
    move || {
        t += 5.0;
        t
    }
}

fn summary_of(j: &dyn EvalJudge, items: &[Item]) -> Summary {
    let refs: Vec<&Item> = items.iter().collect();
    summarize(&run_items(j, &refs, &mut clock(), &mut |_, _| {}), 0.01)
}

#[test]
fn a_perfect_judge_leaks_nothing_blocks_nothing_extra_and_scores_one() {
    let c = corpus();
    let s = summary_of(&Perfect::of(&c), &c);
    assert_eq!(s.n, 10);
    assert_eq!((s.leaks, s.overblocks, s.defers, s.softened), (0, 0, 0, 0));
    assert_eq!(s.leak_rate, 0.0);
    assert_eq!(s.accuracy, 1.0);
    for pv in &s.per_verdict {
        assert_eq!(pv.precision, Some(1.0), "{pv:?}");
        assert_eq!(pv.recall, Some(1.0), "{pv:?}");
    }
    assert_eq!(s.confusion[0], [6, 0, 0, 0]);
    assert_eq!(s.confusion[1], [0, 2, 0, 0]);
    assert_eq!(s.confusion[2], [0, 0, 2, 0]);
}

#[test]
fn always_allow_leaks_every_blocked_item_and_over_blocks_nothing() {
    let c = corpus();
    let s = summary_of(&Always(Verdict::Allow), &c);
    assert_eq!(s.leaks, 4);
    assert_eq!(s.leak_rate, 1.0);
    assert_eq!(s.overblocks, 0);
    assert_eq!(s.overblock_rate, 0.0);
    assert_eq!(s.per_verdict[0].precision, Some(0.6));
    assert_eq!(s.per_verdict[0].recall, Some(1.0));
    assert_eq!(s.per_verdict[2].precision, None);
    assert_eq!(s.per_verdict[2].recall, Some(0.0));
    assert_eq!(s.injection.leaks, 1);
    assert_eq!(s.injection.n, 2);
}

#[test]
fn always_deny_leaks_nothing_and_over_blocks_every_clean_item() {
    let c = corpus();
    let s = summary_of(&Always(Verdict::Deny("no".into())), &c);
    assert_eq!(s.leaks, 0);
    assert_eq!(s.overblocks, 6);
    assert_eq!(s.overblock_rate, 1.0);
    // a deny where a redact was right is not a leak and not an over-block: it is just wrong
    assert_eq!(s.accuracy, 0.2);
    assert_eq!(s.per_verdict[2].precision, Some(0.2));
}

#[test]
fn the_null_judge_defers_everything_which_is_neither_right_nor_wrong() {
    let c = corpus();
    let s = summary_of(&Always(Verdict::Defer), &c);
    assert_eq!(s.defers, 10);
    assert_eq!(s.defer_rate, 1.0);
    assert_eq!((s.leaks, s.overblocks), (0, 0));
    assert_eq!(s.accuracy_decided, None);
    assert!(s.per_verdict.iter().all(|p| p.precision.is_none()));
    assert!(s.calibration.is_none());
}

#[test]
fn a_seeded_random_judge_gives_the_same_report_twice_and_the_matrix_adds_up() {
    let c = corpus();
    let a = summary_of(&Random(Cell::new(42)), &c);
    let b = summary_of(&Random(Cell::new(42)), &c);
    assert_eq!(format!("{a:?}"), format!("{b:?}"));
    let total: usize = a.confusion.iter().flatten().sum();
    assert_eq!(total, 10);
    assert_eq!(
        a.defers,
        a.confusion.iter().map(|row| row[3]).sum::<usize>()
    );
    assert!(a.calibration.is_some());
}

#[test]
fn a_redaction_that_keeps_a_span_is_a_leak_and_a_redact_for_a_deny_is_softened() {
    struct Keeps;
    impl EvalJudge for Keeps {
        fn name(&self) -> String {
            "keeps".into()
        }
        fn evaluate(&self, item: &ReviewItem) -> Judged {
            Judged::of(Verdict::Redact(item.text.clone()))
        }
    }
    struct Scrubs;
    impl EvalJudge for Scrubs {
        fn name(&self) -> String {
            "scrubs".into()
        }
        fn evaluate(&self, _: &ReviewItem) -> Judged {
            Judged::of(Verdict::Redact("[removed]".into()))
        }
    }
    let c = corpus();
    let keeps = summary_of(&Keeps, &c);
    assert_eq!(keeps.leaks, 4, "every redaction kept its span");
    assert_eq!(keeps.redaction_leaks, 4);
    let scrubs = summary_of(&Scrubs, &c);
    assert_eq!(scrubs.leaks, 0);
    assert_eq!(scrubs.softened, 2, "two deny items were only redacted");
    assert_eq!(scrubs.overblocks, 6);
}

#[test]
fn percentiles_use_the_nearest_rank() {
    let v: Vec<f64> = (1..=100).map(f64::from).collect();
    assert_eq!(percentile(&v, 50.0), Some(50.0));
    assert_eq!(percentile(&v, 99.0), Some(99.0));
    assert_eq!(percentile(&v, 100.0), Some(100.0));
    assert_eq!(percentile(&[7.0], 99.0), Some(7.0));
    assert_eq!(percentile(&[], 50.0), None);
    let c = corpus();
    let s = summary_of(&Always(Verdict::Allow), &c);
    assert_eq!(s.latency_p50_ms, Some(5.0));
}

fn rec(truth: Truth, pick: Truth, conf: f64) -> Record {
    let verdict = match pick {
        Truth::Allow => Called::Allow,
        Truth::Redact => Called::Redact,
        Truth::Deny => Called::Deny,
    };
    Record {
        id: "x".into(),
        category: "everyday-chat".into(),
        injection: false,
        truth,
        verdict,
        redaction_leaked: false,
        raw: Some((pick, conf)),
        latency_ms: 1.0,
        tokens_in: None,
        tokens_out: None,
        cost_usd: None,
        error: None,
    }
}

#[test]
fn a_calibrated_judge_has_no_calibration_error_and_an_overconfident_one_does() {
    // 0.95 confident and right 19 times in 20; 0.55 confident and right 11 times in 20
    let mut calibrated = vec![];
    for i in 0..20 {
        let wrong = i == 0;
        calibrated.push(rec(
            Truth::Allow,
            if wrong { Truth::Deny } else { Truth::Allow },
            0.95,
        ));
    }
    for i in 0..20 {
        let wrong = i < 9;
        calibrated.push(rec(
            Truth::Allow,
            if wrong { Truth::Deny } else { Truth::Allow },
            0.55,
        ));
    }
    let c = calibration(&calibrated).unwrap();
    assert!(c.ece.abs() < 1e-9, "{}", c.ece);
    assert_eq!(c.buckets.iter().map(|b| b.n).sum::<usize>(), 40);
    // always 0.9 confident and right half the time: ECE is 0.4
    let over: Vec<Record> = (0..10)
        .map(|i| {
            rec(
                Truth::Allow,
                if i % 2 == 0 {
                    Truth::Allow
                } else {
                    Truth::Deny
                },
                0.9,
            )
        })
        .collect();
    let c = calibration(&over).unwrap();
    assert!((c.ece - 0.4).abs() < 1e-9, "{}", c.ece);
    assert!(calibration(&[]).is_none());
}

#[test]
fn the_threshold_table_trades_leaks_for_deferrals_and_names_the_first_threshold_under_the_target() {
    // two leaks, both at low confidence; the rest right and confident
    let mut rs = vec![
        rec(Truth::Deny, Truth::Allow, 0.55),
        rec(Truth::Redact, Truth::Allow, 0.65),
    ];
    for _ in 0..8 {
        rs.push(rec(Truth::Deny, Truth::Deny, 0.99));
    }
    for _ in 0..10 {
        rs.push(rec(Truth::Allow, Truth::Allow, 0.98));
    }
    let rows = threshold_table(&rs);
    let at = |t: f64| {
        rows.iter()
            .find(|r| (r.threshold - t).abs() < 1e-9)
            .unwrap()
    };
    assert_eq!(at(0.0).leaks, 2);
    assert_eq!(at(0.6).leaks, 1);
    assert_eq!(at(0.7).leaks, 0);
    assert_eq!(at(0.7).deferred, 2);
    assert_eq!(at(0.99).deferred, 12);
    assert_eq!(lowest_threshold(&rows, 0.0), Some(0.7));
    assert_eq!(lowest_threshold(&rows, 0.1), Some(0.6));
    assert_eq!(lowest_threshold(&rows, 1.0), Some(0.0));
}

#[test]
fn categories_are_broken_out_and_injection_is_called_out() {
    let c = corpus();
    let s = summary_of(&Always(Verdict::Allow), &c);
    let chat = s
        .by_category
        .iter()
        .find(|r| r.name == "everyday-chat")
        .unwrap();
    assert_eq!((chat.n, chat.correct, chat.leaks), (3, 3, 0));
    let secret = s
        .by_category
        .iter()
        .find(|r| r.name == "secret-credential")
        .unwrap();
    assert_eq!((secret.n, secret.leaks), (1, 1));
    assert_eq!(s.injection.name, "injection (all)");
    assert_eq!(s.injection.n, 2);
}

#[test]
fn tokens_and_cost_add_up_only_when_a_judge_reports_them() {
    struct Costly;
    impl EvalJudge for Costly {
        fn name(&self) -> String {
            "costly".into()
        }
        fn evaluate(&self, _: &ReviewItem) -> Judged {
            Judged {
                tokens_in: Some(100),
                tokens_out: Some(10),
                cost_usd: Some(0.002),
                ..Judged::of(Verdict::Allow)
            }
        }
    }
    let c = corpus();
    let s = summary_of(&Costly, &c);
    assert_eq!(s.tokens_in, Some(1000));
    assert_eq!(s.tokens_out, Some(100));
    assert!((s.cost_usd.unwrap() - 0.02).abs() < 1e-9);
    let none = summary_of(&Always(Verdict::Allow), &c);
    assert_eq!((none.tokens_in, none.cost_usd), (None, None));
}

#[test]
fn the_report_and_the_json_carry_the_numbers_and_never_the_text() {
    let c = corpus();
    let refs: Vec<&Item> = c.iter().collect();
    let records = run_items(&Always(Verdict::Allow), &refs, &mut clock(), &mut |_, _| {});
    let s = summarize(&records, 0.01);
    let text = crate::judge_eval::report::format(&s, "always-allow", "dev");
    for needle in [
        "confusion",
        "leak",
        "over-block",
        "defer",
        "precision",
        "p50",
        "injection",
        "everyday-chat",
        "Wilson 95 percent",
        "(n=",
        "warning: n is below 30",
    ] {
        assert!(text.contains(needle), "{needle} missing:\n{text}");
    }
    let json = crate::judge_eval::report::to_json(&s, &records, "always-allow", "dev").to_string();
    assert!(json.contains("\"leaks\":4"), "{json}");
    for i in &c {
        assert!(
            !json.contains(&i.text),
            "the JSON holds the text of {}",
            i.id
        );
        assert!(!text.contains(&i.text));
    }
}
