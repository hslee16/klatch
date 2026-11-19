//! `klatch-bench judge-eval cascade-sim --first FIRST.json --second SECOND.json [--thresholds LIST]
//! [--allow-thresholds LIST] [--redact-threshold F]` (#936): the cascade of two judges worked out OFFLINE from two
//! result files the runner wrote with `--out`, with no call to any judge.
//!
//! For each pair of thresholds the cascade's policy ([`CascadePolicy`], the same code the real cascade runs) reads the
//! first judge's recorded pick, probability and confidence for each item. An item the policy accepts takes the verdict
//! that outcome would have (a deny, an allow, or a redaction by placeholder); every other item is escalated and takes
//! the second judge's RECORDED verdict, leak and over-block flags. Latency and cost are summed over the stages that the
//! item would have called.
//!
//! This is a simulation and says so: the second judge is not run again, so its verdict on an escalated item is the one
//! from its own run (a judge is not deterministic, and its prompt there did not know it was a second stage). The
//! thresholds are meant to be chosen on the dev split and scored on the test split; choosing them on the split they are
//! scored on tunes to that split. The two files must come from the same corpus and split, and are refused otherwise.

use crate::judge_eval::corpus::Truth;
use crate::judge_eval::interval::rate_with_interval;
use crate::judge_eval::metrics::Called;
use klatch::domain::{CascadePolicy, Lean, Reading};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::Write;

/// One recorded row of a results file.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: String,
    pub truth: Truth,
    pub verdict: Called,
    pub leak: bool,
    pub overblock: bool,
    pub contested: bool,
    pub pick: Option<Truth>,
    pub confidence: Option<f64>,
    /// Absent in files written before the cascade existed; the confidence stands in (see the report).
    pub probability: Option<f64>,
    pub latency_ms: f64,
    pub cost_usd: Option<f64>,
}

/// A results file, read.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub name: String,
    pub judge: String,
    pub split: String,
    pub rows: Vec<Row>,
}

fn called(word: &str) -> Option<Called> {
    Called::ALL.into_iter().find(|c| c.word() == word)
}

/// Read a results file's text. `name` is only used in messages.
pub fn parse_run(text: &str, name: &str) -> Result<Run, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("{name}: not JSON: {e}"))?;
    let judge = v["judge"]
        .as_str()
        .ok_or_else(|| format!("{name}: no judge name; is it a judge-eval --out file?"))?;
    let split = v["split"]
        .as_str()
        .ok_or_else(|| format!("{name}: no split; is it a judge-eval --out file?"))?;
    let items = v["items"]
        .as_array()
        .ok_or_else(|| format!("{name}: no items; is it a judge-eval --out file?"))?;
    let mut rows = Vec::with_capacity(items.len());
    for (n, it) in items.iter().enumerate() {
        let bad = |what: &str| format!("{name}: item {}: {what}", n + 1);
        let id = it["id"].as_str().ok_or_else(|| bad("no id"))?;
        let truth = it["truth"]
            .as_str()
            .and_then(Truth::parse)
            .ok_or_else(|| bad("bad truth"))?;
        let verdict = it["verdict"]
            .as_str()
            .and_then(called)
            .ok_or_else(|| bad("bad verdict"))?;
        let flag = |k: &str| it[k].as_bool().ok_or_else(|| bad(&format!("no {k}")));
        rows.push(Row {
            id: id.to_string(),
            truth,
            verdict,
            leak: flag("leak")?,
            overblock: flag("overblock")?,
            contested: it["contested"].as_bool().unwrap_or(false),
            pick: it["pick"].as_str().and_then(Truth::parse),
            confidence: it["confidence"].as_f64(),
            probability: it["probability"].as_f64(),
            latency_ms: it["latency_ms"].as_f64().ok_or_else(|| bad("no latency"))?,
            cost_usd: it["cost_usd"].as_f64(),
        });
    }
    let mut seen = std::collections::HashSet::new();
    if let Some(r) = rows.iter().find(|r| !seen.insert(r.id.as_str())) {
        return Err(format!("{name}: item id {} appears twice", r.id));
    }
    Ok(Run {
        name: name.to_string(),
        judge: judge.to_string(),
        split: split.to_string(),
        rows,
    })
}

/// Whether two runs scored the same items: the same split, the same item ids, the same ground truth for each, the same
/// contested flags. The corpus itself is not named in a results file, so this is how a different corpus is told.
pub fn compatible(a: &Run, b: &Run) -> Result<(), String> {
    if a.split != b.split {
        return Err(format!(
            "refusing to combine different splits: {} is {:?}, {} is {:?}",
            a.name, a.split, b.name, b.split
        ));
    }
    let theirs: HashMap<&str, &Row> = b.rows.iter().map(|r| (r.id.as_str(), r)).collect();
    if a.rows.len() != b.rows.len() {
        return Err(format!(
            "refusing to combine files from different corpora or selections: {} has {} items, {} has {}",
            a.name,
            a.rows.len(),
            b.name,
            b.rows.len()
        ));
    }
    for r in &a.rows {
        let Some(o) = theirs.get(r.id.as_str()) else {
            return Err(format!(
                "refusing to combine files from different corpora: item {} of {} is not in {}",
                r.id, a.name, b.name
            ));
        };
        if o.truth != r.truth {
            return Err(format!(
                "refusing to combine: item {} has the truth {} in {} and {} in {} (a different corpus version)",
                r.id,
                r.truth.word(),
                a.name,
                o.truth.word(),
                b.name
            ));
        }
        if o.contested != r.contested {
            return Err(format!(
                "refusing to combine: item {} is contested in only one of the files",
                r.id
            ));
        }
    }
    Ok(())
}

/// The figures of one threshold pair.
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    pub policy: CascadePolicy,
    /// Scored items (the contested ones are in no rate).
    pub n: usize,
    pub correct: usize,
    pub leaks: usize,
    /// Scored items that should not cross.
    pub blocked: usize,
    pub overblocks: usize,
    /// Scored clean items.
    pub clean: usize,
    pub defers: usize,
    pub escalated: usize,
    /// The mean end-to-end latency over every row (the contested ones were sent too).
    pub latency_mean_ms: f64,
    /// The total cost over every row; `None` when neither judge reported one.
    pub cost_usd: Option<f64>,
}

/// What one item comes to.
struct Outcome {
    verdict: Called,
    leak: bool,
    overblock: bool,
    escalated: bool,
    latency_ms: f64,
    cost: Option<f64>,
}

fn add(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (None, None) => None,
        _ => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
    }
}

fn reading(r: &Row) -> Option<Reading> {
    let confidence = r.confidence?;
    Some(Reading {
        lean: match r.pick? {
            Truth::Allow => Lean::Allow,
            Truth::Redact => Lean::Redact,
            Truth::Deny => Lean::Deny,
        },
        probability: r.probability.unwrap_or(confidence),
        confidence,
    })
}

fn tally(rows: &[Outcome], truths: &[(Truth, bool)], policy: CascadePolicy) -> Point {
    let mut p = Point {
        policy,
        n: 0,
        correct: 0,
        leaks: 0,
        blocked: 0,
        overblocks: 0,
        clean: 0,
        defers: 0,
        escalated: 0,
        latency_mean_ms: 0.0,
        cost_usd: None,
    };
    let mut latency = 0.0;
    for (o, (truth, contested)) in rows.iter().zip(truths) {
        latency += o.latency_ms;
        p.cost_usd = add(p.cost_usd, o.cost);
        if *contested {
            continue;
        }
        p.n += 1;
        if *truth == Truth::Allow {
            p.clean += 1;
        } else {
            p.blocked += 1;
        }
        p.leaks += usize::from(o.leak);
        p.overblocks += usize::from(o.overblock);
        p.defers += usize::from(o.verdict == Called::Defer);
        p.escalated += usize::from(o.escalated);
        p.correct += usize::from(o.verdict == Called::of(*truth) && !o.leak);
    }
    if !rows.is_empty() {
        p.latency_mean_ms = latency / rows.len() as f64;
    }
    p
}

/// The verdict a first-stage outcome would have as a final one: a deny never leaks, an allow leaks when the item
/// should not cross, a redaction is a placeholder (the first judge has no text) so it keeps nothing.
fn taken(lean: Lean, truth: Truth) -> (Called, bool, bool) {
    match lean {
        Lean::Allow => (Called::Allow, truth != Truth::Allow, false),
        Lean::Deny => (Called::Deny, false, truth == Truth::Allow),
        Lean::Redact => (Called::Redact, false, truth == Truth::Allow),
    }
}

/// The cascade over the two runs for one policy. The runs must be [`compatible`].
pub fn simulate(first: &Run, second: &Run, policy: &CascadePolicy) -> Point {
    let theirs: HashMap<&str, &Row> = second.rows.iter().map(|r| (r.id.as_str(), r)).collect();
    let mut outcomes = vec![];
    let mut truths = vec![];
    for f in &first.rows {
        let s = theirs[f.id.as_str()];
        let accepted = reading(f).and_then(|r| policy.accepts(&r).map(|_| r.lean));
        let o = match accepted {
            Some(lean) => {
                let (verdict, leak, overblock) = taken(lean, f.truth);
                Outcome {
                    verdict,
                    leak,
                    overblock,
                    escalated: false,
                    latency_ms: f.latency_ms,
                    cost: f.cost_usd,
                }
            }
            None => Outcome {
                verdict: s.verdict,
                leak: s.leak,
                overblock: s.overblock,
                escalated: true,
                latency_ms: f.latency_ms + s.latency_ms,
                cost: add(f.cost_usd, s.cost_usd),
            },
        };
        outcomes.push(o);
        truths.push((f.truth, f.contested));
    }
    tally(&outcomes, &truths, *policy)
}

/// A judge alone, as recorded.
fn alone(run: &Run) -> Point {
    let outcomes: Vec<Outcome> = run
        .rows
        .iter()
        .map(|r| Outcome {
            verdict: r.verdict,
            leak: r.leak,
            overblock: r.overblock,
            escalated: false,
            latency_ms: r.latency_ms,
            cost: r.cost_usd,
        })
        .collect();
    let truths: Vec<(Truth, bool)> = run.rows.iter().map(|r| (r.truth, r.contested)).collect();
    tally(
        &outcomes,
        &truths,
        CascadePolicy::new(None, None, None).unwrap_or_default(),
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub first: String,
    pub second: String,
    /// The deny thresholds to try.
    pub thresholds: Vec<f64>,
    /// The allow thresholds to try (a pair is one of each).
    pub allow_thresholds: Vec<f64>,
    pub redact_threshold: Option<f64>,
}

impl Options {
    pub fn defaults(first: &str, second: &str) -> Options {
        Options {
            first: first.into(),
            second: second.into(),
            thresholds: vec![0.5, 0.7, 0.8, 0.9, 0.95, 0.99],
            allow_thresholds: vec![0.9, 0.95, 0.98, 0.99, 1.0],
            redact_threshold: None,
        }
    }

    pub fn parse(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
        let (mut first, mut second) = (None, None);
        let mut o = Options::defaults("", "");
        let value = |args: &mut dyn Iterator<Item = String>, f: &str| {
            args.next().ok_or_else(|| format!("{f} needs a value"))
        };
        let one = |v: &str, f: &str| {
            v.trim()
                .parse::<f64>()
                .ok()
                .filter(|x| x.is_finite() && (0.0..=1.0).contains(x))
                .ok_or_else(|| format!("{f}: {v:?} is not a number from 0 to 1"))
        };
        let list = |v: &str, f: &str| {
            let xs: Result<Vec<f64>, String> = v.split(',').map(|x| one(x, f)).collect();
            xs
        };
        while let Some(a) = args.next() {
            match a.as_str() {
                "--first" => first = Some(value(&mut args, &a)?),
                "--second" => second = Some(value(&mut args, &a)?),
                "--thresholds" => o.thresholds = list(&value(&mut args, &a)?, &a)?,
                "--allow-thresholds" => o.allow_thresholds = list(&value(&mut args, &a)?, &a)?,
                "--redact-threshold" => {
                    o.redact_threshold = Some(one(&value(&mut args, &a)?, &a)?);
                }
                other => return Err(format!("unknown option {other}")),
            }
        }
        o.first = first.ok_or("--first is required")?;
        o.second = second.ok_or("--second is required")?;
        Ok(o)
    }
}

fn load(path: &str) -> Result<Run, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    parse_run(&text, path)
}

fn line(p: &Point) -> String {
    let cost = p
        .cost_usd
        .map_or_else(|| "no cost reported".to_string(), |c| format!("${c:.2}"));
    format!(
        "accuracy {}; strict leaks {}; over-blocks {}; escalated {}; deferred {}; mean latency {:.0} ms; cost {cost}",
        rate_with_interval(p.correct, p.n),
        rate_with_interval(p.leaks, p.blocked),
        rate_with_interval(p.overblocks, p.clean),
        rate_with_interval(p.escalated, p.n),
        p.defers,
        p.latency_mean_ms,
    )
}

/// The printed report for two compatible runs.
pub fn report(first: &Run, second: &Run, o: &Options) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "judge-eval cascade-sim: first {} ({}), second {} ({}), split {}",
        first.judge, first.name, second.judge, second.name, first.split
    );
    let _ = writeln!(
        out,
        "SIMULATION: nothing was called. Every escalated item takes the second judge's recorded verdicts from {}, \
reused as they were in its own run; the second judge was not run as a second stage, and a judge is not deterministic, \
so a real cascade can differ. Each item's first-stage reading is the recorded pick, probability and confidence of {}.",
        second.name, first.name
    );
    let _ = writeln!(
        out,
        "Thresholds: tune the thresholds on dev and score them on test; picking them on the split they are scored on \
tunes to that split. Intervals are Wilson 95 percent confidence intervals. Contested items are in no rate; every item \
counts for latency and cost."
    );
    if first
        .rows
        .iter()
        .any(|r| r.pick.is_some() && r.probability.is_none())
    {
        let _ = writeln!(
            out,
            "note: {} holds no probability for some items; the confidence stands in, which can only make the \
cascade escalate more than the real one (the confidence of a choice is never above its probability).",
            first.name
        );
    }
    let _ = writeln!(out, "\nbaselines, as recorded");
    let _ = writeln!(out, "  first judge alone:  {}", line(&alone(first)));
    let _ = writeln!(out, "  second judge alone: {}", line(&alone(second)));
    let _ = writeln!(
        out,
        "\nthe cascade (a redact is always escalated{}; an allow needs its probability and confidence at the allow \
threshold)",
        o.redact_threshold.map_or(String::new(), |t| format!(
            " unless its probability reaches {t}"
        ))
    );
    for &deny in &o.thresholds {
        for &allow in &o.allow_thresholds {
            let policy =
                CascadePolicy::new(Some(deny), Some(allow), o.redact_threshold).unwrap_or_default();
            let p = simulate(first, second, &policy);
            let _ = writeln!(out, "  deny {deny:.2} allow {allow:.2}: {}", line(&p));
        }
    }
    let _ = writeln!(
        out,
        "\nlimits: the figures are as good as the two recorded runs (one run each, no run-to-run variance); the \
corpus is synthetic; a threshold that is leak-free on one split may not be on the other (the 2026-11-19 results \
disagreed); pick thresholds on dev, then read the test row once."
    );
    out
}

/// Load, check and report; the whole subcommand without the printing.
pub fn run(o: &Options) -> Result<String, String> {
    let first = load(&o.first)?;
    let second = load(&o.second)?;
    compatible(&first, &second)?;
    Ok(report(&first, &second, o))
}

pub fn main(args: impl Iterator<Item = String>) -> i32 {
    let o = match Options::parse(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!(
                "klatch-bench judge-eval cascade-sim: {e}\n{}",
                super::cli::USAGE
            );
            return 2;
        }
    };
    match run(&o) {
        Ok(text) => {
            print!("{text}");
            0
        }
        Err(e) => {
            eprintln!("klatch-bench judge-eval cascade-sim: {e}");
            2
        }
    }
}

#[cfg(test)]
#[path = "cascade_sim_tests.rs"]
mod tests;
