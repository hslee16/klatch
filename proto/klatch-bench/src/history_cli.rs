//! The command-line side of the history: append a run, print the trend, and make a baseline from recorded runs.
//! The logic is in `klatch_bench::history` (tested there); this only reads and writes the files.

use klatch_bench::history;

/// Where the history file is: `--history`, else the default outside the repository.
pub fn path(option: &Option<String>) -> std::path::PathBuf {
    option
        .as_ref()
        .map(std::path::PathBuf::from)
        .or_else(|| history::default_history_path(&|k| std::env::var(k).ok()))
        .unwrap_or_else(|| super::fail("no history path: set --history or HOME".into()))
}

fn read_runs(path: &std::path::Path) -> Vec<history::RunRecord> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let (runs, skipped) = history::load(&text);
    if skipped > 0 {
        eprintln!(
            "klatch-bench: skipped {skipped} unreadable line(s) in {}",
            path.display()
        );
    }
    runs
}

/// Append this run as one line, creating the directory when needed.
pub fn record(path: &std::path::Path, outcomes: &[klatch_bench::Outcome], partial: bool) {
    use std::io::Write;
    let rec = history::record_of(
        outcomes,
        partial,
        history::now_utc(),
        history::current_sha(),
        history::current_machine(),
    );
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let line = format!("{}\n", history::to_line(&rec));
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| f.write_all(line.as_bytes()));
    match written {
        Ok(()) => println!("run recorded in {}", path.display()),
        Err(e) => super::fail(format!("cannot append to {}: {e}", path.display())),
    }
}

/// Print the trend against the baseline file; whether any metric crept.
pub fn trend(path: &std::path::Path, baseline: &str, last: usize) -> bool {
    let runs = read_runs(path);
    let base = std::fs::read_to_string(baseline)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(serde_json::Value::Null);
    let rows = history::trend(&base, &runs, last);
    print!("{}", history::format_trend(&rows, runs.len()));
    rows.iter().any(|r| r.crept)
}

/// Write a baseline (and its `.meta.json` sidecar) from the last `n` complete recorded runs.
pub fn rebaseline(path: &std::path::Path, n: usize, out: &str) {
    let complete: Vec<_> = read_runs(path)
        .into_iter()
        .filter(|r| !r.partial && r.passed == r.total)
        .collect();
    if n == 0 || complete.len() < n {
        super::fail(format!(
            "need {n} complete passing runs in {}, found {}",
            path.display(),
            complete.len()
        ));
    }
    let runs = &complete[complete.len() - n..];
    let (baseline, noisy) = history::baseline_from_runs(runs);
    let text = serde_json::to_string_pretty(&baseline).unwrap_or_default();
    super::write_file(out, format!("{text}\n"));
    let meta_path = out.strip_suffix(".json").unwrap_or(out).to_string() + ".meta.json";
    let meta =
        serde_json::to_string_pretty(&history::baseline_meta(runs, &noisy)).unwrap_or_default();
    super::write_file(&meta_path, format!("{meta}\n"));
    println!(
        "baseline from {n} runs written to {out} (context in {meta_path}); {} noisy metric(s) kept at their worst",
        noisy.len()
    );
}
