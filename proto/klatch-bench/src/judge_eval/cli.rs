//! `klatch-bench judge-eval --judge <null|rules|claude:<model>|jev[:<model>[:<threshold>]]> [--split dev|test]
//! [--limit N] [--out FILE] [--target-leak F] [--ceiling LABEL] [--budget-usd F] [--corpus FILE] [--scrub FILE]
//! [--claude PATH]`
//!
//! A manual command: the claude judge costs tokens and the Jev judge needs a key, so this never runs in CI or in
//! the gate. The null and rules judges are free and run in the unit tests.

use crate::judge_eval::claude::{ClaudeConfig, ClaudeJudge, RealRunner};
use crate::judge_eval::corpus::{self, Split};
use crate::judge_eval::judges::{EvalJudge, RulesJudge, Seam, parse_scrub};
use crate::judge_eval::metrics::{Record, Summary, summarize};
use crate::judge_eval::report;
use crate::judge_eval::run::run_items;
use klatch::domain::NullJudge;

pub const USAGE: &str = "usage: klatch-bench judge-eval --judge <null|rules|claude:<model>|jev[:<model>[:<threshold>]]> \
[--split dev|test] [--limit N] [--out FILE] [--target-leak F] [--ceiling LABEL] [--budget-usd F] \
[--corpus FILE] [--scrub FILE] [--claude PATH]";

#[derive(Debug, Clone, PartialEq)]
pub enum JudgeSpec {
    Null,
    Rules,
    Claude(String),
    Jev {
        model: Option<String>,
        threshold: Option<f64>,
    },
}

impl JudgeSpec {
    fn parse(s: &str) -> Result<JudgeSpec, String> {
        let parts: Vec<&str> = s.split(':').collect();
        match parts.as_slice() {
            ["null"] => Ok(JudgeSpec::Null),
            ["rules"] => Ok(JudgeSpec::Rules),
            ["claude", m] if !m.is_empty() => Ok(JudgeSpec::Claude(m.to_string())),
            ["jev"] => Ok(JudgeSpec::Jev {
                model: None,
                threshold: None,
            }),
            ["jev", m] if !m.is_empty() => Ok(JudgeSpec::Jev {
                model: Some(m.to_string()),
                threshold: None,
            }),
            ["jev", m, t] if !m.is_empty() => Ok(JudgeSpec::Jev {
                model: Some(m.to_string()),
                threshold: Some(t.parse().map_err(|_| format!("bad Jev threshold {t:?}"))?),
            }),
            _ => Err(format!(
                "unknown judge {s:?}: use null, rules, claude:<model> or jev[:<model>[:<threshold>]]"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub judge: JudgeSpec,
    pub split: Split,
    pub limit: Option<usize>,
    pub out: Option<String>,
    /// The leak rate the threshold table is read against.
    pub target_leak: f64,
    /// The hosted judges see only text strictly below this label. The default lets every corpus label through,
    /// because the corpus is synthetic; `internal` is the production default.
    pub ceiling: String,
    pub budget_usd: f64,
    pub corpus: Option<String>,
    pub scrub: Option<String>,
    pub claude: String,
}

fn value(args: &mut dyn Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("{flag} needs a value"))
}

fn number(v: &str, flag: &str) -> Result<f64, String> {
    v.parse::<f64>()
        .ok()
        .filter(|x| x.is_finite())
        .ok_or_else(|| format!("{flag}: {v:?} is not a number"))
}

impl Options {
    pub fn parse(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
        let mut judge = None;
        let mut o = Options {
            judge: JudgeSpec::Null,
            split: Split::Dev,
            limit: None,
            out: None,
            target_leak: 0.01,
            ceiling: "secret".into(),
            budget_usd: 0.25,
            corpus: None,
            scrub: None,
            claude: "claude".into(),
        };
        while let Some(a) = args.next() {
            let a = a.as_str();
            match a {
                "--judge" => judge = Some(JudgeSpec::parse(&value(&mut args, a)?)?),
                "--split" => {
                    o.split =
                        Split::parse(&value(&mut args, a)?).ok_or("--split is dev or test")?;
                }
                "--limit" => {
                    let v = value(&mut args, a)?;
                    o.limit = Some(
                        v.parse::<usize>()
                            .ok()
                            .filter(|n| *n > 0)
                            .ok_or_else(|| format!("--limit: {v:?} is not a positive count"))?,
                    );
                }
                "--out" => o.out = Some(value(&mut args, a)?),
                "--target-leak" => {
                    o.target_leak = number(&value(&mut args, a)?, a)?;
                    if !(0.0..=1.0).contains(&o.target_leak) {
                        return Err("--target-leak is a rate from 0 to 1".into());
                    }
                }
                "--ceiling" => o.ceiling = value(&mut args, a)?,
                "--budget-usd" => {
                    o.budget_usd = number(&value(&mut args, a)?, a)?;
                    if o.budget_usd <= 0.0 {
                        return Err("--budget-usd must be above zero".into());
                    }
                }
                "--corpus" => o.corpus = Some(value(&mut args, a)?),
                "--scrub" => o.scrub = Some(value(&mut args, a)?),
                "--claude" => o.claude = value(&mut args, a)?,
                other => return Err(format!("unknown option {other}")),
            }
        }
        o.judge = judge.ok_or("--judge is required")?;
        Ok(o)
    }
}

/// A fresh private directory for the claude sessions to run in.
fn workdir() -> Result<std::path::PathBuf, String> {
    let d = std::env::temp_dir().join(format!("klatch-judge-eval-{}", std::process::id()));
    std::fs::create_dir_all(&d).map_err(|e| format!("cannot create {}: {e}", d.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700));
    }
    Ok(d)
}

pub fn build_judge(o: &Options, scrub: &[(String, String)]) -> Result<Box<dyn EvalJudge>, String> {
    match &o.judge {
        JudgeSpec::Null => Ok(Box::new(Seam::new("null", NullJudge))),
        JudgeSpec::Rules => Ok(Box::new(Seam::new("rules", RulesJudge::new(scrub.to_vec())))),
        JudgeSpec::Claude(model) => {
            let config = ClaudeConfig {
                model: model.clone(),
                program: o.claude.clone(),
                budget_usd: o.budget_usd,
                scrub_terms: scrub.iter().map(|(_, t)| t.clone()).collect(),
                ceiling: o.ceiling.clone(),
            };
            let runner = RealRunner {
                timeout: std::time::Duration::from_secs(300),
            };
            let lookup = |k: &str| std::env::var(k).ok();
            Ok(Box::new(ClaudeJudge::new(config, runner, &workdir()?, &lookup)?))
        }
        JudgeSpec::Jev { .. } => Err(
            "jev: transport not available: the HTTP transport for Jev is #871 and is not merged, so nothing was \
             sent. When it lands, build a JevEval with it in judge_eval::cli::build_judge (see the design item for #914)."
                .into(),
        ),
    }
}

/// Load the corpus and the scrub list, run the chosen judge over the chosen items, and summarize.
pub fn run(o: &Options) -> Result<(Summary, Vec<Record>), String> {
    let corpus_path = o
        .corpus
        .as_ref()
        .map_or_else(corpus::default_path, std::path::PathBuf::from);
    let scrub_path = o
        .scrub
        .as_ref()
        .map_or_else(corpus::default_scrub_path, std::path::PathBuf::from);
    let items = corpus::load(&corpus_path)?;
    let scrub = parse_scrub(
        &std::fs::read_to_string(&scrub_path)
            .map_err(|e| format!("cannot read {}: {e}", scrub_path.display()))?,
    )?;
    let chosen = corpus::select(&items, Some(o.split), o.limit);
    let judge = build_judge(o, &scrub)?;
    let started = std::time::Instant::now();
    let mut clock = || started.elapsed().as_secs_f64() * 1000.0;
    let talk = matches!(o.judge, JudgeSpec::Claude(_) | JudgeSpec::Jev { .. });
    let mut progress = |done: usize, total: usize| {
        if talk {
            eprintln!("judge-eval: {done}/{total}");
        }
    };
    let records = run_items(judge.as_ref(), &chosen, &mut clock, &mut progress);
    if matches!(o.judge, JudgeSpec::Claude(_)) {
        let _ = std::fs::remove_dir_all(
            std::env::temp_dir().join(format!("klatch-judge-eval-{}", std::process::id())),
        );
    }
    Ok((summarize(&records, o.target_leak), records))
}

fn judge_name(o: &Options) -> String {
    match &o.judge {
        JudgeSpec::Null => "null".into(),
        JudgeSpec::Rules => "rules".into(),
        JudgeSpec::Claude(m) => format!("claude:{m}"),
        JudgeSpec::Jev { model, .. } => format!("jev:{}", model.as_deref().unwrap_or("?")),
    }
}

/// The subcommand: print the report, write the results file, return the exit code.
pub fn main(args: impl Iterator<Item = String>) -> i32 {
    let o = match Options::parse(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("klatch-bench judge-eval: {e}\n{USAGE}");
            return 2;
        }
    };
    let (summary, records) = match run(&o) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("klatch-bench judge-eval: {e}");
            return 2;
        }
    };
    let name = judge_name(&o);
    print!("{}", report::format(&summary, &name, o.split.word()));
    if let Some(path) = &o.out {
        let json = report::to_json(&summary, &records, &name, o.split.word());
        let text = serde_json::to_string_pretty(&json).unwrap_or_default();
        if let Err(e) = std::fs::write(path, format!("{text}\n")) {
            eprintln!("klatch-bench judge-eval: cannot write {path}: {e}");
            return 2;
        }
        println!("\nresults written to {path}");
    }
    0
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
