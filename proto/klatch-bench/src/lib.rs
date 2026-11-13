//! klatch-bench: scripted scenarios that drive the real `klatch` binary, one temporary store each, with a report
//! and a bug corpus (#262, layers 1 to 3 of the bench). A scenario is a function over a `World`; the runner times it,
//! catches a panic or an error as a failure, and prints a table and a JSON record.

mod scenarios;
mod world;

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
                String::new()
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

/// The klatch binary the workspace builds, next to the test or bench executable's directory. Builds it with cargo
/// when it is missing (running only this crate's tests does not build the other crates' binaries).
pub fn find_binary() -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // The profile directory (target/debug or target/release) is the nearest ancestor with that name; the executable
    // can sit several levels below it, and where depends on the cargo version.
    let dir = exe
        .ancestors()
        .find(|d| {
            matches!(
                d.file_name().and_then(|n| n.to_str()),
                Some("debug" | "release")
            )
        })
        .ok_or("could not find the cargo profile directory above the running executable")?
        .to_path_buf();
    let bin = dir.join(if cfg!(windows) {
        "klatch.exe"
    } else {
        "klatch"
    });
    if !bin.exists() {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let status = std::process::Command::new(cargo)
            .args(["build", "-q", "-p", "klatch-cli"])
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() || !bin.exists() {
            return Err(format!("could not find or build {}", bin.display()));
        }
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

    #[test]
    fn the_whole_bench_passes_against_the_real_binary() {
        let bin = find_binary().expect("the klatch binary");
        let out = run_scenarios(&all(), &bin);
        let failed: Vec<_> = out.iter().filter(|o| !o.passed).collect();
        assert!(failed.is_empty(), "{}", table(&out));
    }
}
