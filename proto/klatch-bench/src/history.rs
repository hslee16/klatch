//! Per-run history and drift : one JSON line per bench run, a robust
//! baseline made from several runs, and a trend view that flags a metric that crept up without any single run failing
//! the 3x rule. Pure functions over text and numbers, so the tests need no real bench.

use serde_json::Value;
use std::collections::BTreeMap;

/// What a run was measured on, so a later comparison knows whether the numbers are comparable.
#[derive(Debug, Clone, PartialEq)]
pub struct Machine {
    pub os: String,
    pub cores: u64,
    /// The one-minute load average when the run started.
    pub load1: f64,
}

/// One bench run: when, on what, and every timing metric as `scenario/metric` in milliseconds.
#[derive(Debug, Clone, PartialEq)]
pub struct RunRecord {
    pub time: String,
    pub sha: String,
    pub passed: usize,
    pub total: usize,
    /// A run restricted by `--layer` or by names: its numbers are history, but never a baseline.
    pub partial: bool,
    pub machine: Machine,
    pub timings: BTreeMap<String, f64>,
}

/// A metric that crept: every one of the last `n` runs was over `factor` times the baseline.
pub const CREEP_FACTOR: f64 = 1.5;
/// A creeping metric must also be at least this many ms over its baseline (5 ms to 9 ms is not drift).
pub const CREEP_MIN_DELTA_MS: f64 = 25.0;
/// How many recent runs the trend looks at.
pub const TREND_RUNS: usize = 5;

/// One metric in the trend view.
#[derive(Debug, Clone, PartialEq)]
pub struct TrendRow {
    pub key: String,
    pub baseline: Option<f64>,
    /// The metric in the last runs that carry it, oldest first.
    pub last: Vec<f64>,
    /// The median of `last` over the baseline.
    pub drift: Option<f64>,
    pub crept: bool,
}

/// Whether a metric is a timing the bench budgets, baselines and records: a name ending in `_ms`. The runner's own
/// store check time (`integrity_ms`, 1 to 3 ms in every scenario) is excluded: nobody acts on it.
pub fn is_timing(name: &str) -> bool {
    name.ends_with("_ms") && name != "integrity_ms"
}

/// The one-minute load average from `uptime` text (macOS "load averages: 1.5 2.0 3.0", Linux "load average: 1.5, 2.0").
pub fn parse_load1(uptime: &str) -> Option<f64> {
    let (_, tail) = uptime.split_once("load average")?;
    let tail = tail.trim_start_matches('s').trim_start_matches(':');
    tail.split(|c: char| c == ',' || c.is_whitespace())
        .find(|w| !w.is_empty())?
        .parse()
        .ok()
}

/// Where the history lives by default, outside the repository: `$KLATCH_BENCH_HISTORY`, else
/// `$XDG_STATE_HOME/klatch-bench/history.jsonl`, else `$HOME/.local/state/klatch-bench/history.jsonl`.
pub fn default_history_path(env: &dyn Fn(&str) -> Option<String>) -> Option<std::path::PathBuf> {
    if let Some(p) = env("KLATCH_BENCH_HISTORY") {
        return Some(p.into());
    }
    let state = env("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| env("HOME").map(|h| std::path::Path::new(&h).join(".local/state")))?;
    Some(state.join("klatch-bench/history.jsonl"))
}

fn command_text(program: &str, args: &[&str]) -> String {
    std::process::Command::new(program)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// The machine this run is on (the system `uname`, the core count, the load right now).
pub fn current_machine() -> Machine {
    Machine {
        os: command_text("uname", &["-sr"]),
        cores: std::thread::available_parallelism().map_or(1, |n| n.get() as u64),
        load1: parse_load1(&command_text("uptime", &[])).unwrap_or(-1.0),
    }
}

/// The time now, from the system `date`, as UTC ISO 8601 (`2026-11-18T20:35:23Z`).
pub fn now_utc() -> String {
    command_text("date", &["-u", "+%Y-%m-%dT%H:%M:%SZ"])
}

/// The short git sha of the working tree the bench runs from, with `+dirty` when tracked files differ.
pub fn current_sha() -> String {
    let sha = command_text("git", &["rev-parse", "--short", "HEAD"]);
    let dirty = !command_text("git", &["status", "--porcelain", "--untracked-files=no"]).is_empty();
    if dirty { format!("{sha}+dirty") } else { sha }
}

/// The timings of a set of outcomes as a run record for the history.
pub fn record_of(
    outcomes: &[crate::Outcome],
    partial: bool,
    time: String,
    sha: String,
    machine: Machine,
) -> RunRecord {
    let mut timings = BTreeMap::new();
    for o in outcomes.iter().filter(|o| o.passed) {
        for (k, v) in o.metrics.iter().filter(|(k, _)| is_timing(k)) {
            timings.insert(format!("{}/{k}", o.name), *v);
        }
    }
    RunRecord {
        time,
        sha,
        passed: outcomes.iter().filter(|o| o.passed).count(),
        total: outcomes.len(),
        partial,
        machine,
        timings,
    }
}

/// The run as one JSON line (no newline).
pub fn to_line(r: &RunRecord) -> String {
    serde_json::json!({
        "time": r.time, "sha": r.sha, "passed": r.passed, "total": r.total, "partial": r.partial,
        "machine": machine_json(&r.machine), "timings": r.timings,
    })
    .to_string()
}

fn machine_json(m: &Machine) -> Value {
    serde_json::json!({"os": m.os, "cores": m.cores, "load1": m.load1})
}

/// A run back from its line; `None` for a line that is not a run record.
pub fn parse_line(line: &str) -> Option<RunRecord> {
    let v: Value = serde_json::from_str(line).ok()?;
    let m = v.get("machine")?;
    let timings = v
        .get("timings")?
        .as_object()?
        .iter()
        .filter_map(|(k, x)| Some((k.clone(), x.as_f64()?)))
        .collect();
    Some(RunRecord {
        time: v.get("time")?.as_str()?.to_string(),
        sha: v.get("sha")?.as_str()?.to_string(),
        passed: v.get("passed")?.as_u64()? as usize,
        total: v.get("total")?.as_u64()? as usize,
        partial: v.get("partial")?.as_bool()?,
        machine: Machine {
            os: m.get("os")?.as_str()?.to_string(),
            cores: m.get("cores")?.as_u64()?,
            load1: m.get("load1")?.as_f64()?,
        },
        timings,
    })
}

/// Every readable run in the file text, in file order, and how many lines were skipped.
pub fn load(text: &str) -> (Vec<RunRecord>, usize) {
    let mut runs = Vec::new();
    let mut skipped = 0;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        match parse_line(line) {
            Some(r) => runs.push(r),
            None => skipped += 1,
        }
    }
    (runs, skipped)
}

/// The median; the mean of the middle two for an even count.
pub fn median(values: &[f64]) -> f64 {
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    match v.len() {
        0 => 0.0,
        n if n % 2 == 1 => v[n / 2],
        n => (v[n / 2 - 1] + v[n / 2]) / 2.0,
    }
}

/// The value one metric takes in the baseline from several runs, and whether it was noisy. The rule: the median;
/// but when the runs disagree (max - min over the median above 0.5, and at least 20 ms apart) the metric is noisy
/// under load and the worst (max) is kept, so a baseline never hides behind a lucky quiet run.
pub fn robust(values: &[f64]) -> (f64, bool) {
    let med = median(values);
    let max = values.iter().copied().fold(f64::MIN, f64::max);
    let min = values.iter().copied().fold(f64::MAX, f64::min);
    let noisy = values.len() > 1 && max - min >= 20.0 && (max - min) / med > 0.5;
    (if noisy { max } else { med }, noisy)
}

fn split_key(key: &str) -> Option<(&str, &str)> {
    key.split_once('/')
}

/// The baseline record (`{"scenario": {"metric": ms}}`) from several complete runs, and the noisy keys.
pub fn baseline_from_runs(runs: &[RunRecord]) -> (Value, Vec<String>) {
    let mut by_key: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for r in runs {
        for (k, v) in &r.timings {
            by_key.entry(k).or_default().push(*v);
        }
    }
    let mut out = serde_json::Map::new();
    let mut noisy = Vec::new();
    for (key, values) in by_key {
        let Some((scenario, metric)) = split_key(key) else {
            continue;
        };
        let (value, is_noisy) = robust(&values);
        if is_noisy {
            noisy.push(key.to_string());
        }
        out.entry(scenario)
            .or_insert_with(|| Value::Object(Default::default()))
            .as_object_mut()
            .expect("an object")
            .insert(metric.to_string(), serde_json::json!(value));
    }
    (Value::Object(out), noisy)
}

fn baseline_map(baseline: &Value) -> BTreeMap<String, f64> {
    let mut m = BTreeMap::new();
    for (s, metrics) in baseline.as_object().into_iter().flatten() {
        for (k, v) in metrics.as_object().into_iter().flatten() {
            if let Some(x) = v.as_f64() {
                m.insert(format!("{s}/{k}"), x);
            }
        }
    }
    m
}

/// The trend of every metric the baseline or the runs know, sorted by key.
pub fn trend(baseline: &Value, runs: &[RunRecord], n: usize) -> Vec<TrendRow> {
    let base = baseline_map(baseline);
    let mut keys: Vec<&String> = base.keys().collect();
    for r in runs {
        keys.extend(r.timings.keys());
    }
    keys.sort();
    keys.dedup();
    keys.into_iter()
        .map(|key| {
            let mut last: Vec<f64> = runs
                .iter()
                .filter_map(|r| r.timings.get(key).copied())
                .collect();
            if last.len() > n {
                last.drain(..last.len() - n);
            }
            let baseline = base.get(key).copied();
            let drift = baseline
                .filter(|b| *b > 0.0 && !last.is_empty())
                .map(|b| median(&last) / b);
            let crept = baseline.is_some_and(|b| {
                last.len() >= n
                    && last
                        .iter()
                        .all(|v| *v > b * CREEP_FACTOR && *v - b >= CREEP_MIN_DELTA_MS)
            });
            TrendRow {
                key: key.clone(),
                baseline,
                last,
                drift,
                crept,
            }
        })
        .collect()
}

/// The trend as text: the metrics that crept first, then the rest with a drift, then a total.
pub fn format_trend(rows: &[TrendRow], runs: usize) -> String {
    if runs == 0 {
        return "trend: no runs recorded yet (run the bench with --record)\n".to_string();
    }
    let line = |r: &TrendRow| {
        let last: Vec<String> = r.last.iter().map(|v| format!("{v:.1}")).collect();
        format!(
            "{:<6} {:<80} base {:>8} drift {:>6} last [{}]\n",
            if r.crept { "CREEP" } else { "" },
            r.key,
            r.baseline.map_or("-".to_string(), |b| format!("{b:.1}")),
            r.drift.map_or("-".to_string(), |d| format!("{d:.2}x")),
            last.join(", ")
        )
    };
    let mut out = String::new();
    for r in rows.iter().filter(|r| r.crept) {
        out.push_str(&line(r));
    }
    for r in rows.iter().filter(|r| !r.crept && !r.last.is_empty()) {
        out.push_str(&line(r));
    }
    let crept = rows.iter().filter(|r| r.crept).count();
    out.push_str(&format!(
        "trend: {} metrics over {runs} recorded runs, {crept} crept (every one of the last {TREND_RUNS} runs over {CREEP_FACTOR}x baseline and {CREEP_MIN_DELTA_MS} ms)\n",
        rows.len()
    ));
    out
}

/// The sidecar that says what a baseline was measured on.
pub fn baseline_meta(runs: &[RunRecord], noisy: &[String]) -> Value {
    serde_json::json!({
        "rule": "per metric the median of the runs; the worst (max) when the runs disagree (spread over 0.5 of the median and at least 20 ms)",
        "sha": runs.first().map(|r| r.sha.clone()).unwrap_or_default(),
        "runs": runs.iter().map(|r| serde_json::json!({
            "time": r.time, "sha": r.sha, "passed": r.passed, "total": r.total,
            "machine": machine_json(&r.machine),
        })).collect::<Vec<_>>(),
        "noisy": noisy,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn machine() -> Machine {
        Machine {
            os: "Darwin 23.3.0".into(),
            cores: 10,
            load1: 1.5,
        }
    }

    fn run(time: &str, timings: &[(&str, f64)]) -> RunRecord {
        RunRecord {
            time: time.into(),
            sha: "abc1234".into(),
            passed: 110,
            total: 110,
            partial: false,
            machine: machine(),
            timings: timings.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        }
    }

    #[test]
    fn a_run_round_trips_through_one_line() {
        let r = run(
            "2026-11-18T20:00:00Z",
            &[("s/hook_ms", 12.5), ("t/x_ms", 3.0)],
        );
        let line = to_line(&r);
        assert!(!line.contains('\n'), "one line: {line}");
        assert_eq!(parse_line(&line), Some(r));
    }

    #[test]
    fn the_line_carries_the_documented_fields() {
        let v: Value = serde_json::from_str(&to_line(&run("T", &[("s/a_ms", 1.0)]))).unwrap();
        for k in [
            "time", "sha", "passed", "total", "partial", "machine", "timings",
        ] {
            assert!(v.get(k).is_some(), "missing {k}: {v}");
        }
        assert_eq!(v["machine"]["cores"], 10);
        assert_eq!(v["timings"]["s/a_ms"], 1.0);
    }

    #[test]
    fn a_damaged_or_foreign_line_is_skipped_and_counted_not_fatal() {
        let good = to_line(&run("T1", &[("s/a_ms", 1.0)]));
        let text = format!("{good}\nnot json\n{{\"x\":1}}\n\n{good}\n");
        let (runs, skipped) = load(&text);
        assert_eq!(runs.len(), 2);
        assert_eq!(skipped, 2, "blank lines are not counted, bad ones are");
    }

    #[test]
    fn the_median_of_three_and_of_two() {
        assert_eq!(median(&[30.0, 10.0, 20.0]), 20.0);
        assert_eq!(median(&[10.0, 20.0]), 15.0);
        assert_eq!(median(&[7.0]), 7.0);
    }

    #[test]
    fn a_steady_metric_takes_the_median() {
        assert_eq!(robust(&[100.0, 110.0, 105.0]), (105.0, false));
    }

    #[test]
    fn a_metric_that_disagrees_with_itself_takes_the_worst_and_is_noisy() {
        assert_eq!(robust(&[100.0, 400.0, 150.0]), (400.0, true));
    }

    #[test]
    fn a_tiny_metric_never_counts_as_noisy_however_wide_its_ratio() {
        // 2 ms to 8 ms is 4x but only 6 ms apart: the median is right
        assert_eq!(robust(&[2.0, 8.0, 3.0]), (3.0, false));
    }

    #[test]
    fn the_baseline_from_runs_is_keyed_by_scenario_and_lists_the_noisy_keys() {
        let runs = [
            run("1", &[("s/a_ms", 100.0), ("s/b_ms", 10.0), ("t/c_ms", 5.0)]),
            run("2", &[("s/a_ms", 500.0), ("s/b_ms", 11.0), ("t/c_ms", 5.0)]),
            run("3", &[("s/a_ms", 150.0), ("s/b_ms", 12.0), ("t/c_ms", 5.0)]),
        ];
        let (b, noisy) = baseline_from_runs(&runs);
        assert_eq!(b["s"]["a_ms"], 500.0);
        assert_eq!(b["s"]["b_ms"], 11.0);
        assert_eq!(b["t"]["c_ms"], 5.0);
        assert_eq!(noisy, vec!["s/a_ms".to_string()]);
    }

    #[test]
    fn a_metric_missing_from_one_run_still_gets_a_baseline_from_the_others() {
        let runs = [
            run("1", &[("s/a_ms", 10.0)]),
            run("2", &[]),
            run("3", &[("s/a_ms", 14.0)]),
        ];
        assert_eq!(baseline_from_runs(&runs).0["s"]["a_ms"], 12.0);
    }

    #[test]
    fn the_sidecar_names_the_machine_the_sha_the_runs_and_the_rule() {
        let runs = [run("1", &[]), run("2", &[]), run("3", &[])];
        let m = baseline_meta(&runs, &["s/a_ms".to_string()]);
        assert_eq!(m["sha"], "abc1234");
        assert_eq!(m["runs"].as_array().unwrap().len(), 3);
        assert_eq!(m["runs"][0]["machine"]["os"], "Darwin 23.3.0");
        assert_eq!(m["runs"][0]["machine"]["load1"], 1.5);
        assert_eq!(m["noisy"][0], "s/a_ms");
        assert!(m["rule"].as_str().unwrap().contains("median"));
    }

    #[test]
    fn the_load_average_is_read_from_macos_and_linux_uptime() {
        assert_eq!(
            parse_load1("20:35  up 14 days,  7:26, 5 users, load averages: 5.61 7.86 6.24"),
            Some(5.61)
        );
        assert_eq!(
            parse_load1(" 10:00:01 up 3 days, 2 users,  load average: 0.42, 0.50, 0.61"),
            Some(0.42)
        );
        assert_eq!(parse_load1("no load here"), None);
    }

    #[test]
    fn the_default_history_is_outside_the_repo_and_the_environment_can_move_it() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| v.to_string())
            }
        };
        let p = |pairs: &'static [(&'static str, &'static str)]| default_history_path(&env(pairs));
        assert_eq!(
            p(&[("KLATCH_BENCH_HISTORY", "/x/h.jsonl"), ("HOME", "/h")]),
            Some("/x/h.jsonl".into())
        );
        assert_eq!(
            p(&[("XDG_STATE_HOME", "/s"), ("HOME", "/h")]),
            Some("/s/klatch-bench/history.jsonl".into())
        );
        assert_eq!(
            p(&[("HOME", "/h")]),
            Some("/h/.local/state/klatch-bench/history.jsonl".into())
        );
        assert_eq!(p(&[]), None);
    }

    #[test]
    fn a_record_of_outcomes_keeps_passing_timings_and_counts_the_verdicts() {
        let out = |name: &str, passed: bool, m: &[(&str, f64)]| crate::Outcome {
            name: name.into(),
            origin: "#1".into(),
            layer: "scale".into(),
            passed,
            millis: 1,
            message: String::new(),
            metrics: m.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        };
        let r = record_of(
            &[
                out(
                    "a",
                    true,
                    &[("x_ms", 4.0), ("integrity_ms", 1.0), ("klatch_calls", 9.0)],
                ),
                out("b", false, &[("y_ms", 7.0)]),
            ],
            true,
            "T".into(),
            "sha".into(),
            machine(),
        );
        assert_eq!((r.passed, r.total, r.partial), (1, 2, true));
        assert_eq!(r.timings.len(), 1, "{:?}", r.timings);
        assert_eq!(r.timings["a/x_ms"], 4.0);
    }

    fn crept_history(values: &[f64]) -> Vec<RunRecord> {
        values
            .iter()
            .enumerate()
            .map(|(i, v)| run(&format!("{i}"), &[("s/a_ms", *v)]))
            .collect()
    }

    #[test]
    fn a_metric_over_one_and_a_half_times_baseline_in_each_of_the_last_five_runs_is_flagged() {
        let base = json!({"s": {"a_ms": 100.0}});
        let rows = trend(
            &base,
            &crept_history(&[100.0, 160.0, 170.0, 180.0, 160.0, 175.0]),
            5,
        );
        assert_eq!(rows.len(), 1);
        assert!(rows[0].crept, "{rows:?}");
        assert_eq!(rows[0].last, vec![160.0, 170.0, 180.0, 160.0, 175.0]);
        assert_eq!(rows[0].drift, Some(1.7));
    }

    #[test]
    fn one_slow_run_is_not_creep() {
        let base = json!({"s": {"a_ms": 100.0}});
        let rows = trend(
            &base,
            &crept_history(&[100.0, 100.0, 400.0, 100.0, 100.0]),
            5,
        );
        assert!(!rows[0].crept);
    }

    #[test]
    fn fewer_than_five_runs_cannot_prove_creep() {
        let base = json!({"s": {"a_ms": 100.0}});
        let rows = trend(&base, &crept_history(&[200.0, 200.0, 200.0]), 5);
        assert!(!rows[0].crept);
        assert_eq!(rows[0].last.len(), 3);
    }

    #[test]
    fn a_small_absolute_rise_is_not_creep_even_at_two_times() {
        let base = json!({"s": {"a_ms": 5.0}});
        let rows = trend(&base, &crept_history(&[10.0; 5]), 5);
        assert!(!rows[0].crept, "5 ms over is under the minimum delta");
    }

    #[test]
    fn a_metric_with_no_baseline_is_listed_without_drift_and_never_flagged() {
        let rows = trend(&json!({}), &crept_history(&[500.0; 5]), 5);
        assert_eq!(rows[0].baseline, None);
        assert_eq!(rows[0].drift, None);
        assert!(!rows[0].crept);
    }

    #[test]
    fn a_baseline_metric_never_run_is_listed_with_no_runs() {
        let rows = trend(&json!({"s": {"a_ms": 100.0}}), &[], 5);
        assert_eq!(rows[0].last, Vec::<f64>::new());
        assert!(!rows[0].crept);
    }

    #[test]
    fn the_trend_only_counts_the_last_n_runs() {
        let base = json!({"s": {"a_ms": 100.0}});
        let rows = trend(
            &base,
            &crept_history(&[900.0, 900.0, 100.0, 100.0, 100.0, 100.0, 100.0]),
            5,
        );
        assert!(!rows[0].crept);
        assert_eq!(rows[0].last, vec![100.0; 5]);
    }

    #[test]
    fn the_text_lists_crept_metrics_first_with_the_rule_and_ends_in_a_total() {
        let base = json!({"s": {"a_ms": 100.0, "b_ms": 100.0}});
        let mut runs = Vec::new();
        for i in 0..5 {
            runs.push(run(
                &format!("{i}"),
                &[("s/a_ms", 100.0), ("s/b_ms", 200.0)],
            ));
        }
        let text = format_trend(&trend(&base, &runs, 5), runs.len());
        let creep = text.find("s/b_ms").expect("b listed");
        let steady = text.find("s/a_ms").expect("a listed");
        assert!(creep < steady, "the crept metric comes first:\n{text}");
        assert!(text.contains("CREEP"), "{text}");
        assert!(text.contains("1 crept"), "{text}");
        assert!(text.contains("2.00x"), "{text}");
    }

    #[test]
    fn with_no_history_the_text_says_so() {
        let text = format_trend(&trend(&json!({}), &[], 5), 0);
        assert!(text.contains("no runs recorded"), "{text}");
    }
}
