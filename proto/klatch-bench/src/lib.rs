//! klatch-bench: scripted scenarios that drive the real `klatch` binary, one temporary store each, with a report
//! and a bug corpus (#262, layers 1 to 3 of the bench). A scenario is a function over a `World`; the runner times it,
//! catches a panic or an error as a failure, and prints a table and a JSON record.

mod harness;
mod scenarios;
mod world;

pub use harness::{FakeHarness, Session};
pub use scenarios::all;
pub use world::World;

use std::time::Duration;

/// Where a scenario comes from: a ticket (a bug that must not come back) or a run or design section it replays.
pub type Origin = &'static str;

/// What a scenario returns: the numbers it measured, or why it failed.
pub type ScenarioResult = Result<Vec<(String, f64)>, String>;

/// One scripted scenario.
pub struct Scenario {
    pub name: &'static str,
    /// The ticket, run or DESIGN section it is taken from (the bug corpus: a regression names its origin).
    pub origin: Origin,
    /// Which layer of the bench it belongs to: `functional`, `scale` or `fault`.
    pub layer: &'static str,
    pub run: fn(&World) -> ScenarioResult,
}

/// What one scenario did.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub name: String,
    pub origin: String,
    pub layer: String,
    pub passed: bool,
    pub millis: u128,
    /// The failure message when it did not pass.
    pub message: String,
    /// Named numbers the scenario measured (for example the number of processes it started).
    pub metrics: Vec<(String, f64)>,
}

/// Run each scenario in its own fresh world. A returned error or a panic is a failure, not an abort.
pub fn run_scenarios(scenarios: &[Scenario], bin: &std::path::Path) -> Vec<Outcome> {
    scenarios
        .iter()
        .map(|s| {
            let world = World::new(bin);
            let start = std::time::Instant::now();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (s.run)(&world)));
            let millis = millis(start.elapsed());
            let (passed, message, mut metrics) = match result {
                Ok(Ok(m)) => (true, String::new(), m),
                Ok(Err(e)) => (false, e, Vec::new()),
                Err(p) => {
                    let m = p
                        .downcast_ref::<&str>()
                        .map(|s| s.to_string())
                        .or_else(|| p.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "panicked".into());
                    (false, format!("panicked: {m}"), Vec::new())
                }
            };
            metrics.push(("klatch_calls".into(), world.calls() as f64));
            Outcome {
                name: s.name.into(),
                origin: s.origin.into(),
                layer: s.layer.into(),
                passed,
                millis,
                message,
                metrics,
            }
        })
        .collect()
}

/// The outcomes as a table for people: one line per scenario, then a total.
pub fn table(outcomes: &[Outcome]) -> String {
    let mut out = String::new();
    for o in outcomes {
        out.push_str(&format!(
            "{:<5} {:<58} {:<10} {:<10} {:>6} ms{}\n",
            if o.passed { "pass" } else { "FAIL" },
            o.name,
            o.layer,
            o.origin,
            o.millis,
            if o.passed {
                let shown: Vec<String> = o
                    .metrics
                    .iter()
                    .filter(|(k, _)| k.ends_with("_ms"))
                    .map(|(k, v)| format!("{k}={v:.0}"))
                    .collect();
                if shown.is_empty() {
                    String::new()
                } else {
                    format!("   {}", shown.join(" "))
                }
            } else {
                format!("   {}", o.message)
            }
        ));
    }
    let passed = outcomes.iter().filter(|o| o.passed).count();
    out.push_str(&format!(
        "{passed} passed, {} failed\n",
        outcomes.len() - passed
    ));
    out
}

/// The outcomes as a JSON record, one object per scenario, with the klatch binary version and a timestamp.
pub fn to_json(outcomes: &[Outcome], bin_version: &str, now: u64) -> String {
    let list: Vec<serde_json::Value> = outcomes
        .iter()
        .map(|o| {
            let metrics: serde_json::Map<String, serde_json::Value> = o
                .metrics
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::json!(v)))
                .collect();
            serde_json::json!({
                "name": o.name, "origin": o.origin, "layer": o.layer, "passed": o.passed,
                "millis": o.millis, "message": o.message, "metrics": metrics,
            })
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "klatch": bin_version, "time": now, "scenarios": list,
    }))
    .expect("a JSON record serialises")
}

/// How one timing metric compares with its recorded baseline.
#[derive(Debug, Clone, PartialEq)]
pub struct Budget {
    pub scenario: String,
    pub metric: String,
    pub value: f64,
    /// The recorded baseline, if there is one.
    pub baseline: Option<f64>,
    /// The most the metric may take before it counts as a regression.
    pub limit: Option<f64>,
    pub status: BudgetStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetStatus {
    Ok,
    /// Beyond the allowed factor over the baseline: the only status that fails the check.
    Regression,
    /// No baseline for it yet.
    New,
}

/// Compare every timing metric (a name ending in `_ms`) of every passing scenario with the baseline
/// (`{"<scenario>": {"<metric>": <ms>}}`). A metric may take `factor` times its baseline, but never less than
/// `floor_ms`, so a fast metric on a slow runner is not a regression.
pub fn check_budgets(
    outcomes: &[Outcome],
    baseline: &serde_json::Value,
    factor: f64,
    floor_ms: f64,
) -> Vec<Budget> {
    let mut out = Vec::new();
    for o in outcomes.iter().filter(|o| o.passed) {
        for (metric, value) in o.metrics.iter().filter(|(k, _)| k.ends_with("_ms")) {
            let recorded = baseline
                .get(&o.name)
                .and_then(|m| m.get(metric))
                .and_then(serde_json::Value::as_f64);
            let limit = recorded.map(|b| (b * factor).max(floor_ms));
            let status = match limit {
                None => BudgetStatus::New,
                Some(l) if *value > l => BudgetStatus::Regression,
                Some(_) => BudgetStatus::Ok,
            };
            out.push(Budget {
                scenario: o.name.clone(),
                metric: metric.clone(),
                value: *value,
                baseline: recorded,
                limit,
                status,
            });
        }
    }
    out
}

/// The budget comparison as text, one line per metric and a total.
pub fn format_budgets(budgets: &[Budget]) -> String {
    let mut out = String::new();
    for b in budgets {
        let verdict = match b.status {
            BudgetStatus::Ok => "ok",
            BudgetStatus::Regression => "REGRESSION",
            BudgetStatus::New => "new (no baseline)",
        };
        out.push_str(&format!(
            "{:<10} {:<42} {:<14} {:>9.1} ms (baseline {}, limit {})\n",
            verdict,
            b.scenario,
            b.metric,
            b.value,
            b.baseline.map_or("-".to_string(), |v| format!("{v:.1}")),
            b.limit.map_or("-".to_string(), |v| format!("{v:.1}")),
        ));
    }
    let regressions = budgets
        .iter()
        .filter(|b| b.status == BudgetStatus::Regression)
        .count();
    out.push_str(&format!(
        "{regressions} regression{}\n",
        if regressions == 1 { "" } else { "s" }
    ));
    out
}

/// The baseline record for these outcomes: each timing metric as measured.
pub fn baseline_of(outcomes: &[Outcome]) -> serde_json::Value {
    let mut all = serde_json::Map::new();
    for o in outcomes.iter().filter(|o| o.passed) {
        let timings: serde_json::Map<String, serde_json::Value> = o
            .metrics
            .iter()
            .filter(|(k, _)| k.ends_with("_ms"))
            .map(|(k, v)| (k.clone(), serde_json::json!(v)))
            .collect();
        if !timings.is_empty() {
            all.insert(o.name.clone(), serde_json::Value::Object(timings));
        }
    }
    serde_json::Value::Object(all)
}

/// The klatch binary the workspace builds, next to the test or bench executable's directory. Builds it with cargo
/// when it is missing (running only this crate's tests does not build the other crates' binaries).
pub fn find_binary() -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // The profile directory (target/debug or target/release) is the nearest ancestor with that name; the executable
    // can sit several levels below it, and where depends on the cargo version.
    let profile = exe
        .ancestors()
        .find(|d| {
            matches!(
                d.file_name().and_then(|n| n.to_str()),
                Some("debug" | "release")
            )
        })
        .ok_or("could not find the cargo profile directory above the running executable")?
        .to_path_buf();
    let target = profile.parent().ok_or("no target directory")?.to_path_buf();
    // The bench drives a test build (`--features test-clock`, which lets it set klatch's clock), built into its own
    // target directory so it never replaces the normal debug binary.
    let dir = target.join("bench-bin");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = std::process::Command::new(cargo)
        .args([
            "build",
            "-q",
            "-p",
            "klatch-cli",
            "--features",
            "test-clock",
            "--target-dir",
        ])
        .arg(&dir)
        .status()
        .map_err(|e| e.to_string())?;
    let bin = dir
        .join(profile.file_name().unwrap_or_default())
        .join(if cfg!(windows) {
            "klatch.exe"
        } else {
            "klatch"
        });
    if !status.success() || !bin.exists() {
        return Err(format!("could not build {}", bin.display()));
    }
    let version = std::process::Command::new(&bin)
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .map_err(|e| e.to_string())?;
    if !version.contains("test-clock") {
        return Err(format!(
            "{} is not a test-clock build: {version}",
            bin.display()
        ));
    }
    Ok(bin)
}

/// The elapsed time of a run, for tests.
pub fn millis(d: Duration) -> u128 {
    d.as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(_w: &World) -> ScenarioResult {
        Ok(vec![("things".into(), 3.0)])
    }
    fn bad(_w: &World) -> ScenarioResult {
        Err("expected 2 got 3".into())
    }
    fn boom(_w: &World) -> ScenarioResult {
        panic!("it blew up")
    }

    fn sample() -> Vec<Scenario> {
        vec![
            Scenario {
                name: "good",
                origin: "#1",
                layer: "functional",
                run: ok,
            },
            Scenario {
                name: "bad",
                origin: "#2",
                layer: "functional",
                run: bad,
            },
            Scenario {
                name: "boom",
                origin: "#3",
                layer: "fault",
                run: boom,
            },
        ]
    }

    fn bin() -> std::path::PathBuf {
        std::path::PathBuf::from("/nonexistent/klatch")
    }

    #[test]
    fn a_passing_failing_and_panicking_scenario_are_each_reported_without_aborting() {
        let out = run_scenarios(&sample(), &bin());
        assert_eq!(out.len(), 3);
        assert!(out[0].passed && out[0].message.is_empty());
        assert_eq!(
            out[0].metrics,
            vec![
                ("things".to_string(), 3.0),
                ("klatch_calls".to_string(), 0.0)
            ],
            "the scenario's own metrics, then the number of klatch processes the runner counted"
        );
        assert!(!out[1].passed && out[1].message.contains("expected 2 got 3"));
        assert!(
            !out[2].passed && out[2].message.contains("it blew up"),
            "{:?}",
            out[2]
        );
    }

    #[test]
    fn each_outcome_carries_the_scenarios_name_origin_and_layer() {
        let out = run_scenarios(&sample(), &bin());
        assert_eq!(
            (
                out[1].name.as_str(),
                out[1].origin.as_str(),
                out[1].layer.as_str()
            ),
            ("bad", "#2", "functional")
        );
    }

    #[test]
    fn the_table_has_a_line_per_scenario_and_a_total() {
        let t = table(&run_scenarios(&sample(), &bin()));
        assert!(
            t.contains("good") && t.contains("bad") && t.contains("boom"),
            "{t}"
        );
        assert!(t.contains("pass") && t.contains("FAIL"), "{t}");
        assert!(t.contains("1 passed, 2 failed"), "{t}");
        assert!(t.contains("#2"), "the origin is shown: {t}");
    }

    #[test]
    fn the_json_record_names_the_binary_version_the_time_and_every_scenario() {
        let json = to_json(&run_scenarios(&sample(), &bin()), "klatch 0.1.0", 1234);
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(v["klatch"], "klatch 0.1.0");
        assert_eq!(v["time"], 1234);
        let list = v["scenarios"].as_array().unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0]["name"], "good");
        assert_eq!(list[0]["passed"], true);
        assert_eq!(list[0]["metrics"]["things"], 3.0);
        assert_eq!(list[1]["passed"], false);
        assert!(list[1]["message"].as_str().unwrap().contains("expected 2"));
    }

    #[test]
    fn every_scenario_has_a_unique_name_an_origin_and_a_known_layer() {
        let all = all();
        assert!(
            all.len() >= 10,
            "the bench has only {} scenarios",
            all.len()
        );
        let mut names: Vec<_> = all.iter().map(|s| s.name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "duplicate scenario names");
        for s in &all {
            assert!(!s.origin.is_empty(), "{} has no origin", s.name);
            assert!(
                ["functional", "scale", "fault"].contains(&s.layer),
                "{}: {}",
                s.name,
                s.layer
            );
        }
    }

    // Not run by `cargo test` (so not in CI): the bench is a local tool. Run it with `cargo run -p klatch-bench` or
    // `cargo test -p klatch-bench -- --ignored`.
    #[test]
    #[ignore = "the bench runs locally, not in CI"]
    fn the_whole_bench_passes_against_the_real_binary() {
        let bin = find_binary().expect("the klatch binary");
        let out = run_scenarios(&all(), &bin);
        let failed: Vec<_> = out.iter().filter(|o| !o.passed).collect();
        assert!(failed.is_empty(), "{}", table(&out));
    }

    fn timed(name: &str, metrics: &[(&str, f64)], passed: bool) -> Outcome {
        Outcome {
            name: name.into(),
            origin: "#1".into(),
            layer: "scale".into(),
            passed,
            millis: 1,
            message: String::new(),
            metrics: metrics.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        }
    }

    #[test]
    fn a_metric_within_the_factor_over_its_baseline_is_ok_and_beyond_it_is_a_regression() {
        let base = serde_json::json!({"board": {"board_ms": 100.0}});
        let ok = check_budgets(
            &[timed("board", &[("board_ms", 290.0)], true)],
            &base,
            3.0,
            50.0,
        );
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].status, BudgetStatus::Ok);
        assert_eq!((ok[0].baseline, ok[0].limit), (Some(100.0), Some(300.0)));
        let bad = check_budgets(
            &[timed("board", &[("board_ms", 301.0)], true)],
            &base,
            3.0,
            50.0,
        );
        assert_eq!(bad[0].status, BudgetStatus::Regression);
    }

    #[test]
    fn the_floor_protects_a_fast_metric_on_a_slow_runner() {
        let base = serde_json::json!({"quick": {"quick_ms": 5.0}});
        let r = check_budgets(
            &[timed("quick", &[("quick_ms", 150.0)], true)],
            &base,
            3.0,
            200.0,
        );
        assert_eq!(
            r[0].status,
            BudgetStatus::Ok,
            "150 ms is under the 200 ms floor"
        );
        assert_eq!(r[0].limit, Some(200.0));
    }

    #[test]
    fn a_metric_with_no_baseline_is_new_and_other_metrics_are_ignored() {
        let base = serde_json::json!({});
        let r = check_budgets(
            &[timed(
                "s",
                &[
                    ("fresh_ms", 10.0),
                    ("klatch_calls", 88.0),
                    ("members", 150.0),
                ],
                true,
            )],
            &base,
            3.0,
            50.0,
        );
        assert_eq!(r.len(), 1, "only the _ms metric is a budget: {r:?}");
        assert_eq!(r[0].status, BudgetStatus::New);
        assert_eq!(r[0].baseline, None);
    }

    #[test]
    fn a_failed_scenario_has_no_budget_check() {
        let r = check_budgets(
            &[timed("s", &[("a_ms", 10.0)], false)],
            &serde_json::json!({}),
            3.0,
            50.0,
        );
        assert!(r.is_empty());
    }

    #[test]
    fn the_baseline_record_keeps_only_timing_metrics_of_passing_scenarios() {
        let b = baseline_of(&[
            timed("board", &[("board_ms", 12.0), ("members", 150.0)], true),
            timed("broken", &[("x_ms", 1.0)], false),
        ]);
        assert_eq!(b["board"]["board_ms"], 12.0);
        assert!(b["board"].get("members").is_none());
        assert!(b.get("broken").is_none());
    }

    #[test]
    fn the_budget_report_has_a_line_per_metric_and_names_regressions_and_new_metrics() {
        let base = serde_json::json!({"a": {"a_ms": 10.0}, "b": {"b_ms": 10.0}});
        let b = check_budgets(
            &[
                timed("a", &[("a_ms", 12.0)], true),
                timed("b", &[("b_ms", 900.0)], true),
                timed("c", &[("c_ms", 5.0)], true),
            ],
            &base,
            3.0,
            50.0,
        );
        let text = format_budgets(&b);
        assert!(
            text.contains("a_ms") && text.contains("b_ms") && text.contains("c_ms"),
            "{text}"
        );
        assert!(
            text.contains("REGRESSION") && text.contains("new"),
            "{text}"
        );
        assert!(text.contains("1 regression"), "{text}");
        assert!(format_budgets(&[]).contains("0 regressions"));
    }

    #[test]
    fn the_table_shows_the_timing_metrics_of_a_passing_scenario() {
        let out = vec![timed(
            "board",
            &[("board_ms", 12.4), ("members", 100.0)],
            true,
        )];
        let t = table(&out);
        assert!(t.contains("board_ms=12") && !t.contains("members="), "{t}");
    }
}
