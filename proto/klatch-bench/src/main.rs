//! `klatch-bench [--layer functional|scale|fault] [--json <file>] [name...]`: run the scripted scenarios against the
//! real binary, print a table, write a JSON record, and exit non-zero when one fails.

fn main() {
    let mut layer: Option<String> = None;
    let mut json: Option<String> = None;
    let mut names: Vec<String> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--layer" => layer = args.next(),
            "--json" => json = args.next(),
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
    if outcomes.iter().any(|o| !o.passed) {
        std::process::exit(1);
    }
}
