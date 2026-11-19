use super::*;
use klatch::domain::CascadePolicy;
use serde_json::{Value, json};

/// A corpus of 12 items: 6 allow, 3 redact, 3 deny, one of the allow items contested.
fn corpus() -> Vec<(&'static str, &'static str, bool)> {
    vec![
        ("a1", "allow", false),
        ("a2", "allow", false),
        ("a3", "allow", false),
        ("a4", "allow", false),
        ("a5", "allow", false),
        ("a6", "allow", true),
        ("r1", "redact", false),
        ("r2", "redact", false),
        ("r3", "redact", false),
        ("d1", "deny", false),
        ("d2", "deny", false),
        ("d3", "deny", false),
    ]
}

struct Say {
    verdict: &'static str,
    pick: Option<&'static str>,
    probability: Option<f64>,
    confidence: Option<f64>,
    latency: f64,
    cost: Option<f64>,
}

/// One results file as the runner writes it (`--out`), the rows built from a rule per item.
fn file(judge: &str, split: &str, say: impl Fn(&str, &str) -> Say) -> Value {
    let items: Vec<Value> = corpus()
        .into_iter()
        .map(|(id, truth, contested)| {
            let s = say(id, truth);
            let leak = truth != "allow" && s.verdict == "allow";
            let overblock = truth == "allow" && matches!(s.verdict, "redact" | "deny");
            json!({
                "id": id, "category": "c", "truth": truth, "verdict": s.verdict,
                "leak": leak, "data_leak": leak, "overblock": overblock, "contested": contested,
                "stage": null, "probability": s.probability,
                "pick": s.pick, "confidence": s.confidence,
                "latency_ms": s.latency, "tokens_in": null, "tokens_out": null,
                "cost_usd": s.cost, "error": null
            })
        })
        .collect();
    json!({"judge": judge, "split": split, "summary": {}, "items": items})
}

fn perfect_say(_: &str, truth: &str) -> Say {
    Say {
        verdict: match truth {
            "allow" => "allow",
            "redact" => "redact",
            _ => "deny",
        },
        pick: Some(match truth {
            "allow" => "allow",
            "redact" => "redact",
            _ => "deny",
        }),
        probability: Some(1.0),
        confidence: Some(1.0),
        latency: 3000.0,
        cost: Some(0.01),
    }
}

fn perfect(split: &str) -> Run {
    parse_run(
        &file("claude:sonnet", split, perfect_say).to_string(),
        "second.json",
    )
    .unwrap()
}

/// Allows everything, always sure of itself.
fn leaky(split: &str, p: f64) -> Run {
    let f = file("jev:jev-1.14.0", split, |_, _| Say {
        verdict: "allow",
        pick: Some("allow"),
        probability: Some(p),
        confidence: Some(p),
        latency: 100.0,
        cost: None,
    });
    parse_run(&f.to_string(), "first.json").unwrap()
}

fn policy(deny: f64, allow: f64) -> CascadePolicy {
    CascadePolicy::new(Some(deny), Some(allow), None).unwrap()
}

#[test]
fn a_confident_wrong_first_stage_allow_is_a_leak_the_simulator_shows() {
    let (first, second) = (leaky("dev", 0.99), perfect("dev"));
    // the allow threshold is below the first judge's 0.99: its allow is taken, and 6 items that should not cross do
    let p = simulate(&first, &second, &policy(0.9, 0.98));
    assert_eq!((p.leaks, p.blocked), (6, 6));
    assert_eq!(p.escalated, 0);
    // above it, everything is escalated and the second judge catches them all
    let p = simulate(&first, &second, &policy(0.9, 1.0));
    assert_eq!(p.leaks, 0);
    assert_eq!(p.escalated, 11);
    assert_eq!(p.correct, p.n);
}

#[test]
fn contested_items_are_in_no_rate() {
    let p = simulate(&leaky("dev", 0.99), &perfect("dev"), &policy(0.9, 1.0));
    assert_eq!(p.n, 11);
    assert_eq!(p.clean, 5);
    assert_eq!(p.blocked, 6);
}

#[test]
fn every_first_stage_redact_is_escalated_unless_the_policy_accepts_one() {
    // a first judge that is right and certain about everything, redactions included
    let first = parse_run(
        &file("jev:jev-1.14.0", "dev", |i, t| Say {
            latency: 100.0,
            cost: None,
            ..perfect_say(i, t)
        })
        .to_string(),
        "first.json",
    )
    .unwrap();
    let second = perfect("dev");
    let p = simulate(&first, &second, &policy(0.9, 0.98));
    // exactly the 3 redactions are escalated
    assert_eq!(p.escalated, 3);
    assert_eq!(p.correct, p.n);
    // accepting a redact at the first stage (a placeholder) escalates none
    let p = simulate(
        &first,
        &second,
        &CascadePolicy::new(Some(0.9), Some(0.98), Some(0.9)).unwrap(),
    );
    assert_eq!(p.escalated, 0);
    assert_eq!(p.leaks, 0);
}

#[test]
fn a_first_stage_deny_of_a_clean_item_is_an_over_block() {
    let first = parse_run(
        &file("jev:jev-1.14.0", "dev", |_, _| Say {
            verdict: "deny",
            pick: Some("deny"),
            probability: Some(0.99),
            confidence: Some(0.98),
            latency: 100.0,
            cost: None,
        })
        .to_string(),
        "first.json",
    )
    .unwrap();
    let p = simulate(&first, &perfect("dev"), &policy(0.9, 0.98));
    assert_eq!(p.overblocks, 5);
    assert_eq!(p.leaks, 0);
    assert_eq!(p.escalated, 0);
}

#[test]
fn a_second_stage_that_defers_leaves_the_cascade_deferring() {
    let second = parse_run(
        &file("claude:sonnet", "dev", |_, _| Say {
            verdict: "defer",
            pick: None,
            probability: None,
            confidence: None,
            latency: 3000.0,
            cost: Some(0.0),
        })
        .to_string(),
        "second.json",
    )
    .unwrap();
    let p = simulate(&leaky("dev", 0.5), &second, &policy(0.9, 0.98));
    assert_eq!(p.defers, 11);
    assert_eq!(p.leaks, 0, "a deferred item is held, never allowed");
    assert_eq!(p.correct, 0);
}

#[test]
fn latency_and_cost_are_summed_over_the_stages_actually_called() {
    let (first, second) = (leaky("dev", 0.99), perfect("dev"));
    let none = simulate(&first, &second, &policy(0.9, 0.98));
    assert_eq!(none.latency_mean_ms, 100.0);
    assert_eq!(none.cost_usd, None, "neither stage reported a cost");
    let all = simulate(&first, &second, &policy(0.9, 1.0));
    // every row, the contested one included, is below the allow threshold of 1.0, so all 12 are escalated and paid for
    assert_eq!(all.latency_mean_ms, 3100.0);
    assert!(
        (all.cost_usd.unwrap() - 0.12).abs() < 1e-9,
        "{:?}",
        all.cost_usd
    );
}

#[test]
fn the_probability_stands_in_with_the_confidence_when_a_file_has_none() {
    let f = file("jev:jev-1.14.0", "dev", |_, _| Say {
        verdict: "deny",
        pick: Some("deny"),
        probability: None,
        confidence: Some(0.95),
        latency: 100.0,
        cost: None,
    });
    let first = parse_run(&f.to_string(), "first.json").unwrap();
    assert!(first.rows.iter().all(|r| r.probability.is_none()));
    let p = simulate(&first, &perfect("dev"), &policy(0.9, 0.98));
    assert_eq!(
        p.escalated, 0,
        "the confidence of 0.95 reached the deny threshold"
    );
    assert!(
        report(&first, &perfect("dev"), &Options::defaults("f", "s")).contains("no probability")
    );
}

#[test]
fn files_of_different_splits_or_corpora_are_refused() {
    let (dev, test) = (perfect("dev"), perfect("test"));
    let e = compatible(&dev, &test).unwrap_err();
    assert!(e.contains("split"), "{e}");
    // a different set of items
    let mut other = file("claude:sonnet", "dev", perfect_say);
    other["items"].as_array_mut().unwrap().pop();
    let other = parse_run(&other.to_string(), "other.json").unwrap();
    let e = compatible(&dev, &other).unwrap_err();
    assert!(e.contains("items") || e.contains("corpus"), "{e}");
    let mut renamed = file("claude:sonnet", "dev", perfect_say);
    renamed["items"][0]["id"] = json!("zz");
    let renamed = parse_run(&renamed.to_string(), "renamed.json").unwrap();
    let e = compatible(&dev, &renamed).unwrap_err();
    assert!(e.contains("zz") || e.contains("a1"), "{e}");
    // the same id with a different ground truth
    let mut relabelled = file("claude:sonnet", "dev", perfect_say);
    relabelled["items"][0]["truth"] = json!("deny");
    let relabelled = parse_run(&relabelled.to_string(), "relabelled.json").unwrap();
    let e = compatible(&dev, &relabelled).unwrap_err();
    assert!(e.contains("a1") && e.contains("truth"), "{e}");
    assert!(compatible(&dev, &perfect("dev")).is_ok());
}

#[test]
fn the_main_path_refuses_mismatched_files_and_prints_nothing_else() {
    let dir = std::env::temp_dir().join(format!("klatch-sim-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let write = |name: &str, v: &Value| {
        let p = dir.join(name);
        std::fs::write(&p, v.to_string()).unwrap();
        p.to_string_lossy().into_owned()
    };
    let dev = write("dev.json", &file("jev:jev-1.14.0", "dev", perfect_say));
    let test = write("test.json", &file("claude:sonnet", "test", perfect_say));
    let o = Options::defaults(&dev, &test);
    let e = run(&o).unwrap_err();
    assert!(e.contains("split"), "{e}");
    let ok = write("dev2.json", &file("claude:sonnet", "dev", perfect_say));
    let out = run(&Options::defaults(&dev, &ok)).unwrap();
    assert!(out.contains("SIMULATION"));
    let e = run(&Options::defaults(
        &dev,
        &dir.join("missing.json").to_string_lossy(),
    ))
    .unwrap_err();
    assert!(e.contains("missing.json"), "{e}");
    let bad = dir.join("bad.json");
    std::fs::write(&bad, "not json").unwrap();
    let e = run(&Options::defaults(&dev, &bad.to_string_lossy())).unwrap_err();
    assert!(e.contains("bad.json"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_report_says_it_reuses_the_second_judges_recorded_verdicts_and_gives_intervals() {
    let text = report(
        &leaky("dev", 0.99),
        &perfect("dev"),
        &Options::defaults("first.json", "second.json"),
    );
    assert!(text.contains("SIMULATION"), "{text}");
    assert!(text.contains("recorded verdicts"), "{text}");
    assert!(text.contains("tune the thresholds on dev"), "{text}");
    assert!(text.contains("Wilson"), "{text}");
    assert!(
        text.contains("jev:jev-1.14.0") && text.contains("claude:sonnet"),
        "{text}"
    );
    assert!(text.contains("second judge alone"), "{text}");
    assert!(text.contains("first judge alone"), "{text}");
    // a row per threshold pair: the defaults are 6 deny thresholds by 5 allow thresholds
    let rows = text.lines().filter(|l| l.starts_with("  deny ")).count();
    assert_eq!(rows, 30, "{text}");
    assert!(text.contains('['), "an interval is shown");
}

#[test]
fn a_random_first_judge_is_neutralised_by_strict_thresholds() {
    // a first stage that picks pseudo-randomly with a pseudo-random confidence
    let state = std::cell::Cell::new(0x2545F4914F6CDD1Du64);
    let next = || {
        let mut x = state.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        state.set(x);
        x
    };
    let f = file("jev:jev-1.14.0", "dev", |_, _| {
        let pick = ["allow", "redact", "deny"][(next() % 3) as usize];
        let c = (next() % 1000) as f64 / 1000.0;
        Say {
            verdict: pick,
            pick: Some(pick),
            probability: Some(c),
            confidence: Some(c * 0.9),
            latency: 100.0,
            cost: None,
        }
    });
    let first = parse_run(&f.to_string(), "first.json").unwrap();
    let second = perfect("dev");
    // with every threshold at 1.0 only a certain reading is taken, so the cascade is the second judge alone
    let strict = simulate(&first, &second, &policy(1.0, 1.0));
    let alone = simulate(
        &first,
        &second,
        &CascadePolicy::new(None, None, None).unwrap(),
    );
    assert_eq!(strict.leaks, alone.leaks);
    assert_eq!(strict.correct, alone.correct);
    // at zero every reading is taken except a redact, which is always escalated
    let redacts = first
        .rows
        .iter()
        .filter(|r| !r.contested && r.pick == Some(Truth::Redact))
        .count();
    let loose = simulate(&first, &second, &policy(0.0, 0.0));
    assert_eq!(loose.escalated, redacts);
}

// the command line

fn args(a: &[&str]) -> Result<Options, String> {
    Options::parse(a.iter().map(|s| s.to_string()))
}

#[test]
fn the_options_are_read_and_checked() {
    let o = args(&["--first", "a.json", "--second", "b.json"]).unwrap();
    assert_eq!(o.first, "a.json");
    assert_eq!(o.second, "b.json");
    assert!(o.thresholds.contains(&0.9) && o.allow_thresholds.contains(&0.98));
    let o = args(&[
        "--first",
        "a.json",
        "--second",
        "b.json",
        "--thresholds",
        "0.5, 0.9",
        "--allow-thresholds",
        "0.95,1",
        "--redact-threshold",
        "0.9",
    ])
    .unwrap();
    assert_eq!(o.thresholds, vec![0.5, 0.9]);
    assert_eq!(o.allow_thresholds, vec![0.95, 1.0]);
    assert_eq!(o.redact_threshold, Some(0.9));
    for bad in [
        vec!["--second", "b.json"],
        vec!["--first", "a.json"],
        vec!["--first", "a", "--second", "b", "--thresholds", "0.5,x"],
        vec!["--first", "a", "--second", "b", "--thresholds", "1.5"],
        vec!["--first", "a", "--second", "b", "--thresholds", ""],
        vec!["--first", "a", "--second", "b", "--bogus"],
    ] {
        assert!(args(&bad).is_err(), "{bad:?}");
    }
}
