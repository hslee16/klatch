//! `klatch-bench judge-eval --judge <null|rules|claude:<model>|jev[:<model>[:<threshold>]]> [--split dev|test]
//! [--limit N] [--out FILE] [--target-leak F] [--ceiling LABEL] [--budget-usd F] [--corpus FILE]
//! [--origin design|search] [--scrub FILE] [--claude PATH]`, and
//! `--judge cascade:<first>,<second>` with `[--deny-threshold F] [--allow-threshold F] [--redact-threshold F]` (#936);
//! `klatch-bench judge-eval cascade-sim` combines two recorded result files offline (see `cascade_sim`).
//!
//! A manual command: the claude judge costs tokens and the Jev judge needs a key, so this never runs in CI or in
//! the gate. The null and rules judges are free and run in the unit tests.

use crate::judge_eval::cascade::CascadeEval;
use crate::judge_eval::claude::{ClaudeConfig, ClaudeJudge, RealRunner};
use crate::judge_eval::corpus::{self, Split};
use crate::judge_eval::judges::{EvalJudge, RulesJudge, Seam, parse_scrub};
use crate::judge_eval::metrics::{Record, Summary, summarize};
use crate::judge_eval::report;
use crate::judge_eval::run::run_items;
use klatch::domain::{CascadePolicy, NullJudge};

pub const USAGE: &str = "usage: klatch-bench judge-eval --judge <null|rules|claude:<model>|jev[:<model>[:<threshold>]]> \
[--split dev|test] [--limit N] [--out FILE] [--target-leak F] [--ceiling LABEL] [--budget-usd F] \
[--corpus FILE] [--origin design|search] [--scrub FILE] [--claude PATH]
       --judge cascade:<first>,<second> takes [--deny-threshold F] [--allow-threshold F] [--redact-threshold F]
       klatch-bench judge-eval cascade-sim --first FIRST.json --second SECOND.json [--thresholds LIST] [--allow-thresholds LIST]";

#[derive(Debug, Clone, PartialEq)]
pub enum JudgeSpec {
    Null,
    Rules,
    Claude(String),
    Jev {
        model: Option<String>,
        threshold: Option<f64>,
    },
    /// Two judges that report a pick (Jev or claude): the first for what it is sure of, the second for the rest.
    Cascade {
        first: Box<JudgeSpec>,
        second: Box<JudgeSpec>,
    },
}

impl JudgeSpec {
    fn parse(s: &str) -> Result<JudgeSpec, String> {
        if let Some(pair) = s.strip_prefix("cascade:") {
            let stages: Vec<&str> = pair.split(',').collect();
            let [first, second] = stages.as_slice() else {
                return Err(format!(
                    "cascade needs exactly two judges: cascade:<first>,<second>, got {s:?}"
                ));
            };
            let stage = |t: &str| match JudgeSpec::parse(t)? {
                j @ (JudgeSpec::Claude(_) | JudgeSpec::Jev { .. }) => Ok(Box::new(j)),
                _ => Err(format!(
                    "cascade stage {t:?}: a stage must be claude:<model> or jev:<model>[:<threshold>] (the first \
                     reports the pick the policy reads; null and rules report none)"
                )),
            };
            return Ok(JudgeSpec::Cascade {
                first: stage(first)?,
                second: stage(second)?,
            });
        }
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
    /// Hard corpus only: score just the items of one origin.
    pub origin: Option<corpus::Origin>,
    /// Cascade only (#936): see [`Options::cascade_policy`].
    pub deny_threshold: Option<f64>,
    pub allow_threshold: Option<f64>,
    pub redact_threshold: Option<f64>,
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
            origin: None,
            deny_threshold: None,
            allow_threshold: None,
            redact_threshold: None,
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
                "--origin" => {
                    o.origin = Some(
                        corpus::Origin::parse(&value(&mut args, a)?)
                            .ok_or("--origin is design or search")?,
                    );
                }
                "--deny-threshold" => o.deny_threshold = Some(number(&value(&mut args, a)?, a)?),
                "--allow-threshold" => o.allow_threshold = Some(number(&value(&mut args, a)?, a)?),
                "--redact-threshold" => {
                    o.redact_threshold = Some(number(&value(&mut args, a)?, a)?);
                }
                other => return Err(format!("unknown option {other}")),
            }
        }
        o.judge = judge.ok_or("--judge is required")?;
        let set = o.deny_threshold.is_some()
            || o.allow_threshold.is_some()
            || o.redact_threshold.is_some();
        if set && !matches!(o.judge, JudgeSpec::Cascade { .. }) {
            return Err(
                "--deny-threshold, --allow-threshold and --redact-threshold apply to a cascade only"
                    .into(),
            );
        }
        Ok(o)
    }

    /// The cascade policy: each flag as given; without `--deny-threshold` the first judge's own threshold in the spec
    /// (`jev:<model>:0.9`), else the documented default; without `--allow-threshold` the default, raised to the deny
    /// threshold when that is higher (an allow is never easier than a deny); a redact is escalated unless
    /// `--redact-threshold` is given.
    pub fn cascade_policy(&self) -> Result<CascadePolicy, String> {
        let default = CascadePolicy::default();
        let from_spec = match &self.judge {
            JudgeSpec::Cascade { first, .. } => match first.as_ref() {
                JudgeSpec::Jev { threshold, .. } => *threshold,
                _ => None,
            },
            _ => None,
        };
        let deny = self.deny_threshold.or(from_spec).or(default.deny_threshold);
        let allow = self.allow_threshold.or_else(|| {
            default
                .allow_threshold
                .map(|a| deny.map_or(a, |d| a.max(d)))
        });
        CascadePolicy::new(deny, allow, self.redact_threshold)
    }
}

/// The threshold the Jev judge uses when the spec gives none (the Jev API has no default; this is the bench's).
pub const DEFAULT_JEV_THRESHOLD: f64 = 0.8;

/// The Jev settings from the spec: the model must be a pinned version, so a spec without one is refused.
pub fn jev_config(
    model: Option<&str>,
    threshold: Option<f64>,
) -> Result<klatch::jev::JevConfig, String> {
    let model = model.ok_or(
        "jev: name a pinned model, for example --judge jev:jev-1.14.0[:0.8] (never latest)",
    )?;
    klatch::jev::JevConfig::new(model, threshold.unwrap_or(DEFAULT_JEV_THRESHOLD))
        .map_err(|e| format!("jev: {e}"))
}

#[cfg(feature = "jev-http")]
fn build_jev(
    model: Option<&str>,
    threshold: Option<f64>,
    ceiling: &str,
) -> Result<Box<dyn EvalJudge>, String> {
    use crate::judge_eval::jev::JevEval;
    use klatch::jev::retry::{RetryPolicy, SystemClock, ThreadSleeper};
    let config = jev_config(model, threshold)?;
    if klatch::jev::ApiKey::from_env().is_none() {
        return Err(format!(
            "jev: set {} (the key is read from the environment only)",
            klatch::jev::KEY_ENV
        ));
    }
    let key: Box<dyn Fn() -> Option<String>> =
        Box::new(|| klatch::jev::ApiKey::from_env().map(|k| k.expose().to_string()));
    Ok(Box::new(JevEval::with_client(
        klatch::jev::http::HttpTransport::new(),
        config,
        ceiling,
        |c| {
            c.with_key_source(key).with_retry(
                RetryPolicy::default(),
                Box::new(SystemClock::new()),
                Box::new(ThreadSleeper),
                Box::new(klatch::jev::http::OsJitter),
            )
        },
    )))
}

#[cfg(not(feature = "jev-http"))]
fn build_jev(
    model: Option<&str>,
    threshold: Option<f64>,
    _ceiling: &str,
) -> Result<Box<dyn EvalJudge>, String> {
    jev_config(model, threshold)?;
    Err("jev: the real transport is not built in: build with --features jev-http (cargo run -p klatch-bench \
         --features jev-http -- judge-eval --judge jev:<model>); nothing was sent"
        .into())
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
    build_spec(&o.judge, o, scrub)
}

fn build_spec(
    spec: &JudgeSpec,
    o: &Options,
    scrub: &[(String, String)],
) -> Result<Box<dyn EvalJudge>, String> {
    match spec {
        JudgeSpec::Cascade { first, second } => Ok(Box::new(CascadeEval::new(
            build_spec(first, o, scrub)?,
            build_spec(second, o, scrub)?,
            o.cascade_policy()?,
        ))),
        JudgeSpec::Null => Ok(Box::new(Seam::new("null", NullJudge))),
        JudgeSpec::Rules => Ok(Box::new(Seam::new(
            "rules",
            RulesJudge::new(scrub.to_vec()),
        ))),
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
            Ok(Box::new(ClaudeJudge::new(
                config,
                runner,
                &workdir()?,
                &lookup,
            )?))
        }
        JudgeSpec::Jev { model, threshold } => build_jev(model.as_deref(), *threshold, &o.ceiling),
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
    let items: Vec<corpus::Item> = match o.origin {
        None => items,
        Some(origin) => items
            .into_iter()
            .filter(|i| i.origin == Some(origin))
            .collect(),
    };
    let chosen = corpus::select(&items, Some(o.split), o.limit);
    if o.origin.is_none()
        && chosen
            .iter()
            .any(|i| i.origin == Some(corpus::Origin::Search))
    {
        eprintln!(
            "judge-eval: this selection mixes design and search items; search items were kept because one judge \
             got them wrong, so score them apart with --origin (see judge-eval/README-hard.md)"
        );
    }
    let judge = build_judge(o, &scrub)?;
    let started = std::time::Instant::now();
    let mut clock = || started.elapsed().as_secs_f64() * 1000.0;
    let talk = !matches!(o.judge, JudgeSpec::Null | JudgeSpec::Rules);
    let mut progress = |done: usize, total: usize| {
        if talk {
            eprintln!("judge-eval: {done}/{total}");
        }
    };
    let records = run_items(judge.as_ref(), &chosen, &mut clock, &mut progress);
    if talk {
        let _ = std::fs::remove_dir_all(
            std::env::temp_dir().join(format!("klatch-judge-eval-{}", std::process::id())),
        );
    }
    Ok((summarize(&records, o.target_leak), records))
}

fn spec_name(spec: &JudgeSpec) -> String {
    match spec {
        JudgeSpec::Null => "null".into(),
        JudgeSpec::Rules => "rules".into(),
        JudgeSpec::Claude(m) => format!("claude:{m}"),
        JudgeSpec::Jev { model, .. } => format!("jev:{}", model.as_deref().unwrap_or("?")),
        JudgeSpec::Cascade { first, second } => {
            format!("cascade:{},{}", spec_name(first), spec_name(second))
        }
    }
}

fn judge_name(o: &Options) -> String {
    spec_name(&o.judge)
}

/// The subcommand: print the report, write the results file, return the exit code.
pub fn main(args: impl Iterator<Item = String>) -> i32 {
    let mut args = args.peekable();
    if args.peek().map(String::as_str) == Some("cascade-sim") {
        args.next();
        return crate::judge_eval::cascade_sim::main(args);
    }
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
