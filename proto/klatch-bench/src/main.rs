//! `klatch-bench [--layer functional|scale|fault] [--json <file>] [name...]`: run the scripted scenarios against the
//! real binary, print a table, write a JSON record, and exit non-zero when one fails.

/// What the command line asked for.
struct Options {
    layer: Option<String>,
    json: Option<String>,
    names: Vec<String>,
    budgets: Option<String>,
    save_baseline: Option<String>,
    factor: f64,
    floor: f64,
}

impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Self {
        let mut o = Options {
            layer: None,
            json: None,
            names: Vec::new(),
            budgets: None,
            save_baseline: None,
            factor: 3.0,
            floor: 200.0,
        };
        while let Some(a) = args.next() {
            match a.as_str() {
                "--layer" => o.layer = args.next(),
                "--json" => o.json = args.next(),
                "--budgets" => o.budgets = args.next(),
                "--save-baseline" => o.save_baseline = args.next(),
                "--factor" => o.factor = number(args.next(), o.factor),
                "--floor" => o.floor = number(args.next(), o.floor),
                other => o.names.push(other.to_string()),
            }
        }
        o
    }

    /// Whether a scenario was asked for: in the layer, and named (or no names were given).
    fn wants(&self, s: &klatch_bench::Scenario) -> bool {
        let in_layer = self.layer.as_deref().is_none_or(|l| s.layer == l);
        let named = self.names.is_empty() || self.names.iter().any(|n| s.name.contains(n.as_str()));
        in_layer && named
    }
}

fn number(value: Option<String>, default: f64) -> f64 {
    value.and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn fail(message: String) -> ! {
    eprintln!("klatch-bench: {message}");
    std::process::exit(2);
}

fn write_file(path: &str, text: String) {
    if let Err(e) = std::fs::write(path, text) {
        fail(format!("cannot write {path}: {e}"));
    }
}

fn version_of(bin: &std::path::Path) -> String {
    std::process::Command::new(bin)
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Compare the timings with the saved baseline and print the report; whether any scenario regressed.
fn check_budgets(outcomes: &[klatch_bench::Outcome], path: &str, o: &Options) -> bool {
    let baseline = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(serde_json::Value::Null);
    let report = klatch_bench::check_budgets(outcomes, &baseline, o.factor, o.floor);
    print!("\n{}", klatch_bench::format_budgets(&report));
    report
        .iter()
        .any(|b| b.status == klatch_bench::BudgetStatus::Regression)
}

fn main() {
    let options = Options::parse(std::env::args().skip(1));
    let bin = klatch_bench::find_binary().unwrap_or_else(|e| {
        eprintln!("klatch-bench: {e}");
        std::process::exit(2);
    });
    let chosen: Vec<_> = klatch_bench::all()
        .into_iter()
        .filter(|s| options.wants(s))
        .collect();
    let outcomes = klatch_bench::run_scenarios(&chosen, &bin);
    print!("{}", klatch_bench::table(&outcomes));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    if let Some(path) = &options.json {
        write_file(
            path,
            klatch_bench::to_json(&outcomes, &version_of(&bin), now),
        );
    }
    if let Some(path) = &options.save_baseline {
        let text =
            serde_json::to_string_pretty(&klatch_bench::baseline_of(&outcomes)).unwrap_or_default();
        write_file(path, format!("{text}\n"));
        println!("baseline written to {path}");
    }
    let regressed = options
        .budgets
        .as_deref()
        .is_some_and(|path| check_budgets(&outcomes, path, &options));
    if outcomes.iter().any(|o| !o.passed) || regressed {
        std::process::exit(1);
    }
}
