//! klatch-bench: scripted scenarios that drive the real `klatch` binary, one temporary store each, with a report
//! and a bug corpus (#262, layers 1 to 3 of the bench). A scenario is a function over a `World`; the runner times it,
//! catches a panic or an error as a failure, and prints a table and a JSON record.

mod harness;
pub mod history;
pub mod judge_eval;
pub mod mp;
pub mod mp_run;
mod scenarios;
mod world;

/// The scratch directory of the tests and of each world, shared with the klatch crate by a path attribute (#995).
#[allow(dead_code)]
#[path = "../../klatch/src/store/test_dir.rs"]
mod test_dir;

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
            // a scenario that passed must also leave a sound store (#621); its time is reported apart
            let checked = std::time::Instant::now();
            let result = match result {
                Ok(Ok(mut m)) => Ok(world.check_integrity().map(|()| {
                    if world.has_store() {
                        m.push((
                            "integrity_ms".into(),
                            checked.elapsed().as_secs_f64() * 1000.0,
                        ));
                    }
                    m
                })),
                other => other,
            };
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
/// What follows a scenario's timings on its line: the timing metrics when it passed, else why it failed.
fn detail(o: &Outcome) -> String {
    if !o.passed {
        return format!("   {}", o.message);
    }
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
}

/// One line of the table: the verdict, the name, layer and origin, the time, and the detail.
fn table_line(o: &Outcome) -> String {
    format!(
        "{:<5} {:<58} {:<10} {:<10} {:>6} ms{}\n",
        if o.passed { "pass" } else { "FAIL" },
        o.name,
        o.layer,
        o.origin,
        o.millis,
        detail(o)
    )
}

pub fn table(outcomes: &[Outcome]) -> String {
    let mut out: String = outcomes.iter().map(table_line).collect();
    let passed = outcomes.iter().filter(|o| o.passed).count();
    let failed: Vec<&str> = outcomes
        .iter()
        .filter(|o| !o.passed)
        .map(|o| o.name.as_str())
        .collect();
    // The names follow the counts, so a run that failed once and passed on the rerun still says which scenario (#701);
    // the "N passed, N failed" prefix is what pr-sweep reads.
    let named = if failed.is_empty() {
        String::new()
    } else {
        format!(" (failed: {})", failed.join(", "))
    };
    out.push_str(&format!(
        "{passed} passed, {} failed{named}\n",
        failed.len()
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
        for (metric, value) in o.metrics.iter().filter(|(k, _)| history::is_timing(k)) {
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
            .filter(|(k, _)| history::is_timing(k))
            .map(|(k, v)| (k.clone(), serde_json::json!(v)))
            .collect();
        if !timings.is_empty() {
            all.insert(o.name.clone(), serde_json::Value::Object(timings));
        }
    }
    serde_json::Value::Object(all)
}

/// The cargo arguments that build the test-clock binary in the same profile as the running bench: a release bench that
/// built a debug binary would then run whatever stale release binary was left in the directory (#729).
fn build_args(profile: &str, dir: &std::path::Path) -> Vec<String> {
    let mut args: Vec<String> = ["build", "-q", "-p", "klatch", "--features", "test-clock"]
        .iter()
        .map(|a| a.to_string())
        .collect();
    if profile == "release" {
        args.push("--release".into());
    }
    args.push("--target-dir".into());
    args.push(dir.display().to_string());
    args
}

/// The cargo arguments that build `klatch-server` as shipped (no test clock: the channel never reads it) in the same
/// profile as the running bench, into its own target directory so it never meets the klatch copies (#1078).
fn server_build_args(profile: &str, dir: &std::path::Path) -> Vec<String> {
    let mut args: Vec<String> = ["build", "-q", "-p", "klatch-server"]
        .iter()
        .map(|a| a.to_string())
        .collect();
    if profile == "release" {
        args.push("--release".into());
    }
    args.push("--target-dir".into());
    args.push(dir.display().to_string());
    args
}

/// Runs `build` under an exclusive file lock (so no two threads or processes build at once) and returns a private copy
/// of the binary it produced, named by its content and renamed into place atomically. Cargo removes and re-links the
/// built binary on every build, even a fresh one, so a test that had been handed the built path could find it gone
/// while another test or test binary was rebuilding (#752). The copy is never touched by cargo.
fn obtain_binary(
    dir: &std::path::Path,
    built: &std::path::Path,
    build: impl FnOnce() -> Result<(), String>,
) -> Result<std::path::PathBuf, String> {
    obtain_binary_pruning(dir, built, build, remove_file)
}

fn remove_file(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::remove_file(path)
}

/// How many content-named copies (the current one included) stay in `bench-bin` (#1026).
const KEEP_COPIES: usize = 3;

/// Removes old `klatch-<hash>` copies from `dir`, keeping the newest `keep` by modification time and always keeping
/// `current`. Best effort: any failure is logged to stderr and the rest carries on, since a stale copy costs only disk.
fn prune_copies(
    dir: &std::path::Path,
    current: &std::path::Path,
    keep: usize,
    remove: impl Fn(&std::path::Path) -> std::io::Result<()>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!(
                "klatch-bench: could not list {} to prune ({e})",
                dir.display()
            );
            return;
        }
    };
    let mut copies: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("klatch-"))
        .filter_map(|e| {
            let modified = e.metadata().and_then(|m| m.modified()).ok()?;
            Some((modified, e.path()))
        })
        .collect();
    copies.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (_, path) in copies.into_iter().skip(keep) {
        if path == current {
            continue;
        }
        if let Err(e) = remove(&path) {
            eprintln!("klatch-bench: could not prune {} ({e})", path.display());
        }
    }
}

fn obtain_binary_pruning(
    dir: &std::path::Path,
    built: &std::path::Path,
    build: impl FnOnce() -> Result<(), String>,
    remove: impl Fn(&std::path::Path) -> std::io::Result<()>,
) -> Result<std::path::PathBuf, String> {
    use std::hash::{Hash, Hasher};
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(".build.lock"))
        .map_err(|e| e.to_string())?;
    lock.lock().map_err(|e| e.to_string())?;
    // The lock is released when `lock` drops, on every exit path.
    build()?;
    let bytes =
        std::fs::read(built).map_err(|e| format!("could not build {} ({e})", built.display()))?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    let name = format!(
        "klatch-{:016x}{}",
        hasher.finish(),
        if cfg!(windows) { ".exe" } else { "" }
    );
    let stable = dir.join(name);
    if !stable.exists() {
        let tmp = dir.join(format!(".tmp-{}", std::process::id()));
        std::fs::copy(built, &tmp).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &stable).map_err(|e| e.to_string())?;
    }
    prune_copies(dir, &stable, KEEP_COPIES, remove);
    Ok(stable)
}

/// The klatch binary the workspace builds, next to the test or bench executable's directory. Builds it with cargo
/// when it is missing (running only this crate's tests does not build the other crates' binaries).
/// Built once per process; every later call returns the same private copy.
pub fn find_binary() -> Result<std::path::PathBuf, String> {
    static BINARY: std::sync::OnceLock<Result<std::path::PathBuf, String>> =
        std::sync::OnceLock::new();
    BINARY.get_or_init(build_binary).clone()
}

/// The `klatch-server` binary the workspace builds, as a private copy (see `find_binary`). Built on first use, so a
/// bench run that selects no channel scenario never builds it (#1078).
pub fn find_server_binary() -> Result<std::path::PathBuf, String> {
    static BINARY: std::sync::OnceLock<Result<std::path::PathBuf, String>> =
        std::sync::OnceLock::new();
    BINARY.get_or_init(build_server_binary).clone()
}

fn build_server_binary() -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
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
    let dir = target.join("bench-bin-server");
    let name = profile
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("debug");
    let built = dir.join(name).join(if cfg!(windows) {
        "klatch-server.exe"
    } else {
        "klatch-server"
    });
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let args = server_build_args(name, &dir);
    obtain_binary(&dir, &built, || {
        let status = std::process::Command::new(cargo)
            .args(args)
            .status()
            .map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!(
                "could not build {} (cargo {status}; its own error is printed above)",
                built.display()
            ))
        }
    })
}

fn build_binary() -> Result<std::path::PathBuf, String> {
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
    let built = dir
        .join(profile.file_name().unwrap_or_default())
        .join(if cfg!(windows) {
            "klatch.exe"
        } else {
            "klatch"
        });
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let args = build_args(
        profile
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("debug"),
        &dir,
    );
    let bin = obtain_binary(&dir, &built, || {
        let status = std::process::Command::new(cargo)
            .args(args)
            .status()
            .map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!(
                "could not build {} (cargo {status}; its own error is printed above)",
                built.display()
            ))
        }
    })?;
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

    /// A stand-in for cargo's uplift, which removes the binary and links a fresh one in (#752).
    fn fake_uplift(bin: &std::path::Path, content: &str, slow: Duration) {
        let _ = std::fs::remove_file(bin);
        std::thread::sleep(slow);
        std::fs::write(bin, content).unwrap();
    }

    #[test]
    fn a_binary_in_use_survives_a_later_rebuild_that_removes_the_built_one() {
        let tmp = crate::test_dir::TestDir::new("752-a");
        let dir = tmp.join("bench-bin");
        std::fs::create_dir_all(&tmp).unwrap();
        let bin = tmp.join("klatch");
        let first = obtain_binary(&dir, &bin, || {
            fake_uplift(&bin, "v1", Duration::ZERO);
            Ok(())
        })
        .unwrap();
        let (in_build_tx, in_build_rx) = std::sync::mpsc::channel();
        let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
        let (dir2, bin2) = (dir.clone(), bin.clone());
        let second = std::thread::spawn(move || {
            obtain_binary(&dir2, &bin2, || {
                let _ = std::fs::remove_file(&bin2);
                in_build_tx.send(()).unwrap();
                go_rx.recv().unwrap();
                std::fs::write(&bin2, "v2").unwrap();
                Ok(())
            })
        });
        // The rebuild has removed the built file and has not yet written the new one.
        in_build_rx.recv().unwrap();
        assert!(!bin.exists());
        assert!(first.exists(), "the binary in use vanished mid-rebuild");
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "v1");
        go_tx.send(()).unwrap();
        let second = second.join().unwrap().unwrap();
        assert!(first.exists() && second.exists() && first != second);
    }

    #[test]
    fn builds_are_serialised_so_none_overlaps_another() {
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        let tmp = crate::test_dir::TestDir::new("752-b");
        let dir = tmp.join("bench-bin");
        std::fs::create_dir_all(&tmp).unwrap();
        let bin = tmp.join("klatch");
        let running = std::sync::Arc::new(AtomicUsize::new(0));
        let overlaps = std::sync::Arc::new(AtomicUsize::new(0));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let (dir, bin) = (dir.clone(), bin.clone());
                let (running, overlaps) = (running.clone(), overlaps.clone());
                std::thread::spawn(move || {
                    obtain_binary(&dir, &bin, || {
                        if running.fetch_add(1, SeqCst) != 0 {
                            overlaps.fetch_add(1, SeqCst);
                        }
                        fake_uplift(&bin, "v1", Duration::from_millis(60));
                        running.fetch_sub(1, SeqCst);
                        Ok(())
                    })
                    .unwrap()
                })
            })
            .collect();
        let paths: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(overlaps.load(SeqCst), 0);
        // Same content, same private copy: nothing accumulates.
        assert!(paths.iter().all(|p| p == &paths[0] && p.exists()));
    }

    fn aged_copy(dir: &std::path::Path, name: &str, age_secs: u64) -> std::path::PathBuf {
        let p = dir.join(name);
        let f = std::fs::File::create(&p).unwrap();
        f.set_modified(std::time::SystemTime::now() - Duration::from_secs(age_secs))
            .unwrap();
        p
    }

    fn fake_build(bin: &std::path::Path) -> impl FnOnce() -> Result<(), String> + '_ {
        move || {
            fake_uplift(bin, "current", Duration::ZERO);
            Ok(())
        }
    }

    #[test]
    fn old_copies_beyond_the_cap_are_pruned_and_the_newest_and_current_stay() {
        let tmp = crate::test_dir::TestDir::new("1026-a");
        let dir = tmp.join("bench-bin");
        std::fs::create_dir_all(&dir).unwrap();
        let bin = tmp.join("klatch");
        let old: Vec<_> = (0..5)
            .map(|i| aged_copy(&dir, &format!("klatch-old{i}"), 1000 + i * 100))
            .collect();
        let cur = obtain_binary(&dir, &bin, fake_build(&bin)).unwrap();
        assert!(cur.exists());
        // The current copy is newest; the two newest old ones stay with it.
        assert!(old[0].exists() && old[1].exists());
        assert!(
            old[2..].iter().all(|p| !p.exists()),
            "old copies not pruned"
        );
        assert!(dir.join(".build.lock").exists());
    }

    #[test]
    fn the_current_copy_is_never_pruned_even_when_it_is_the_oldest() {
        let tmp = crate::test_dir::TestDir::new("1026-b");
        let dir = tmp.join("bench-bin");
        std::fs::create_dir_all(&dir).unwrap();
        let cur = aged_copy(&dir, "klatch-cur", 9999);
        for i in 0..5 {
            aged_copy(&dir, &format!("klatch-new{i}"), 10 + i);
        }
        prune_copies(&dir, &cur, 3, remove_file);
        assert!(cur.exists());
        assert!(dir.join("klatch-new0").exists() && dir.join("klatch-new2").exists());
        assert!(!dir.join("klatch-new3").exists() && !dir.join("klatch-new4").exists());
    }

    #[test]
    fn a_prune_error_does_not_fail_the_build() {
        let tmp = crate::test_dir::TestDir::new("1026-c");
        let dir = tmp.join("bench-bin");
        std::fs::create_dir_all(&dir).unwrap();
        let bin = tmp.join("klatch");
        for i in 0..5 {
            aged_copy(&dir, &format!("klatch-old{i}"), 1000 + i);
        }
        let got = obtain_binary_pruning(&dir, &bin, fake_build(&bin), |_| {
            Err(std::io::Error::other("denied"))
        })
        .unwrap();
        assert!(got.exists());
    }

    #[test]
    fn a_missing_directory_to_prune_does_not_panic() {
        let tmp = crate::test_dir::TestDir::new("1026-d");
        prune_copies(&tmp.join("none"), &tmp.join("klatch-x"), 3, remove_file);
    }

    #[test]
    fn a_failed_build_is_an_error_not_a_missing_file() {
        let tmp = crate::test_dir::TestDir::new("752-c");
        std::fs::create_dir_all(&tmp).unwrap();
        let err =
            obtain_binary(&tmp.join("bench-bin"), &tmp.join("klatch"), || Ok(())).unwrap_err();
        assert!(err.contains("could not build"), "{err}");
    }

    #[test]
    fn every_scenario_is_reported_even_when_one_fails_or_panics() {
        assert_eq!(run_scenarios(&sample(), &bin()).len(), 3);
    }

    #[test]
    fn a_passing_scenario_reports_its_metrics_and_the_klatch_calls_the_runner_counted() {
        let out = run_scenarios(&sample(), &bin());
        assert!(out[0].passed && out[0].message.is_empty());
        let metrics = vec![
            ("things".to_string(), 3.0),
            ("klatch_calls".to_string(), 0.0),
        ];
        assert_eq!(out[0].metrics, metrics);
    }

    #[test]
    fn a_failing_scenario_reports_why() {
        let out = run_scenarios(&sample(), &bin());
        assert!(!out[1].passed && out[1].message.contains("expected 2 got 3"));
    }

    #[test]
    fn a_panicking_scenario_is_reported_as_a_failure_with_the_panic_message() {
        let out = run_scenarios(&sample(), &bin());
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

    fn sample_table() -> String {
        table(&run_scenarios(&sample(), &bin()))
    }

    #[test]
    fn the_table_has_a_line_per_scenario_with_its_verdict_and_origin() {
        let t = sample_table();
        assert!(
            t.contains("good") && t.contains("bad") && t.contains("boom"),
            "{t}"
        );
        assert!(t.contains("pass") && t.contains("FAIL"), "{t}");
        assert!(t.contains("#2"), "the origin is shown: {t}");
    }

    #[test]
    fn the_table_ends_with_a_total() {
        assert!(sample_table().contains("1 passed, 2 failed"));
    }

    #[test]
    fn the_total_names_each_failed_scenario_so_a_one_line_log_says_which() {
        let t = sample_table();
        let last = t.lines().last().expect("a total line");
        assert_eq!(last, "1 passed, 2 failed (failed: bad, boom)", "{t}");
    }

    #[test]
    fn a_clean_total_keeps_the_plain_form_the_sweep_reads() {
        let out = run_scenarios(&sample()[..1], &bin());
        assert_eq!(table(&out).lines().last(), Some("1 passed, 0 failed"));
    }

    fn sample_json() -> serde_json::Value {
        let json = to_json(&run_scenarios(&sample(), &bin()), "klatch 0.1.0", 1234);
        serde_json::from_str(&json).expect("valid JSON")
    }

    #[test]
    fn the_json_record_names_the_binary_version_and_the_time() {
        let v = sample_json();
        assert_eq!(v["klatch"], "klatch 0.1.0");
        assert_eq!(v["time"], 1234);
    }

    #[test]
    fn the_json_record_lists_every_scenario_with_its_verdict_and_metrics() {
        let v = sample_json();
        let list = v["scenarios"].as_array().unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0]["name"], "good");
        assert_eq!(list[0]["metrics"]["things"], 3.0);
        assert_eq!(list[1]["passed"], false);
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

    #[test]
    fn the_store_check_time_is_not_a_budget_baseline_or_history_metric() {
        let o = [timed("s", &[("integrity_ms", 2.0), ("hook_ms", 5.0)], true)];
        let budgets = check_budgets(&o, &serde_json::json!({}), 3.0, 200.0);
        assert_eq!(budgets.len(), 1, "{budgets:?}");
        assert_eq!(budgets[0].metric, "hook_ms");
        assert_eq!(baseline_of(&o), serde_json::json!({"s": {"hook_ms": 5.0}}));
        assert!(!history::is_timing("integrity_ms"));
        assert!(history::is_timing("hook_ms"));
        assert!(!history::is_timing("klatch_calls"));
    }

    /// The scenarios where a latency is the point report it (run with `--ignored`, against the real binary).
    #[test]
    #[ignore = "the bench runs locally, not in CI"]
    fn the_latency_scenarios_report_their_timings() {
        let wanted: &[(&str, &[&str])] = &[
            (
                "a_session_start_with_a_big_board",
                &["session_start_hook_ms"],
            ),
            ("a_long_digest_stays_under", &["digest_hook_ms"]),
            ("a_claimed_path_stops_a_second_editor", &["claim_block_ms"]),
            ("an_idle_session_is_woken_by_a_pointer", &["wake_ms"]),
            ("new_session_joins_without_flood", &["session_start_ms"]),
            (
                "post_tool_use_hook_prints_json_context",
                &["post_tool_use_ms"],
            ),
            ("first_opens_race", &["race_ms"]),
            ("old_databases_migrate", &["migrate_ms"]),
            ("rotating_the_key_keeps_every_message", &["rotate_ms"]),
            (
                "a_backup_is_encrypted",
                &["backup_ms", "restore_ms", "export_ms"],
            ),
        ];
        let bin = find_binary().expect("the klatch binary");
        for (name, metrics) in wanted {
            let chosen: Vec<_> = all()
                .into_iter()
                .filter(|s| s.name.contains(name))
                .collect();
            assert_eq!(chosen.len(), 1, "{name} matches {} scenarios", chosen.len());
            let out = run_scenarios(&chosen, &bin);
            assert!(out[0].passed, "{name}: {}", out[0].message);
            for m in *metrics {
                assert!(
                    out[0].metrics.iter().any(|(k, _)| k == m),
                    "{name} does not report {m}: {:?}",
                    out[0].metrics
                );
            }
        }
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

#[cfg(test)]
mod build_args_tests {
    use super::{build_args, server_build_args};
    use std::path::Path;

    #[test]
    fn the_server_build_is_klatch_server_in_the_same_profile_and_its_own_target_dir() {
        let rel = server_build_args("release", Path::new("/t/bench-bin-server"));
        assert!(rel.contains(&"klatch-server".to_string()), "{rel:?}");
        assert!(rel.contains(&"--release".to_string()), "{rel:?}");
        assert!(
            !rel.contains(&"test-clock".to_string()),
            "the server is built as shipped: {rel:?}"
        );
        let dbg = server_build_args("debug", Path::new("/t/bench-bin-server"));
        assert!(!dbg.contains(&"--release".to_string()), "{dbg:?}");
        assert_eq!(dbg.last().map(String::as_str), Some("/t/bench-bin-server"));
    }

    #[test]
    fn a_release_bench_builds_the_release_binary() {
        let args = build_args("release", Path::new("/t/bench-bin"));
        assert!(args.contains(&"--release".to_string()), "{args:?}");
    }

    #[test]
    fn a_debug_bench_builds_the_debug_binary() {
        let args = build_args("debug", Path::new("/t/bench-bin"));
        assert!(!args.contains(&"--release".to_string()), "{args:?}");
        assert_eq!(args.last().map(String::as_str), Some("/t/bench-bin"));
    }
}
