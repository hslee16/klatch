//! `klatch-bench [--layer functional|scale|fault] [--json <file>] [name...]`: run the scripted scenarios against the
//! real binary, print a table, write a JSON record, and exit non-zero when one fails.

fn main() {
    let mut layer: Option<String> = None;
    let mut json: Option<String> = None;
    let mut names: Vec<String> = Vec::new();
    let mut budgets: Option<String> = None;
    let mut save_baseline: Option<String> = None;
    let mut factor = 3.0_f64;
    let mut floor = 200.0_f64;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--layer" => layer = args.next(),
            "--json" => json = args.next(),
            "--budgets" => budgets = args.next(),
            "--save-baseline" => save_baseline = args.next(),
            "--factor" => factor = args.next().and_then(|v| v.parse().ok()).unwrap_or(factor),
            "--floor" => floor = args.next().and_then(|v| v.parse().ok()).unwrap_or(floor),
            other => names.push(other.to_string()),
        }
    }
    let bin = match klatch_bench::find_binary() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("klatch-bench: {e}");
            std::process::exit(2);
        }
    };
    let chosen: Vec<_> = klatch_bench::all()
        .into_iter()
        .filter(|s| layer.as_deref().is_none_or(|l| s.layer == l))
        .filter(|s| names.is_empty() || names.iter().any(|n| s.name.contains(n.as_str())))
        .collect();
    let outcomes = klatch_bench::run_scenarios(&chosen, &bin);
    print!("{}", klatch_bench::table(&outcomes));
    let version = std::process::Command::new(&bin)
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let record = klatch_bench::to_json(&outcomes, &version, now);
    if let Some(path) = json
        && let Err(e) = std::fs::write(&path, record)
    {
        eprintln!("klatch-bench: cannot write {path}: {e}");
        std::process::exit(2);
    }
    if let Some(path) = save_baseline {
        let text =
            serde_json::to_string_pretty(&klatch_bench::baseline_of(&outcomes)).unwrap_or_default();
        if let Err(e) = std::fs::write(&path, format!("{text}\n")) {
            eprintln!("klatch-bench: cannot write {path}: {e}");
            std::process::exit(2);
        }
        println!("baseline written to {path}");
    }
    let mut regressed = false;
    if let Some(path) = budgets {
        let baseline = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(serde_json::Value::Null);
        let report = klatch_bench::check_budgets(&outcomes, &baseline, factor, floor);
        print!("\n{}", klatch_bench::format_budgets(&report));
        regressed = report
            .iter()
            .any(|b| b.status == klatch_bench::BudgetStatus::Regression);
    }
    if outcomes.iter().any(|o| !o.passed) || regressed {
        std::process::exit(1);
    }
}
