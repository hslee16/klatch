//! `klatch judge run` (#48, #517): drain the review queue through the configured judge, once with `--once` or in a
//! resident loop. The judge is the local rules judge (credential scanner and scrub list): it denies a match and
//! defers everything else, so clean text stays queued for the reviewer. An opt-in hosted judge (#960, a Claude model
//! reached through the operator's own `claude` binary) runs only when the operator names both `--hosted-judge` and
//! `--hosted-ceiling`; it is for our own rooms and evaluation, never a default. Without those flags nothing in this
//! command sends text anywhere. Like the other launcher commands it is refused for a worker and from inside a coding-agent
//! session, because a verdict is policy and an agent must not rule on its own messages.

use super::*;
use clap::Subcommand;
use klatch::domain::{Breaker, Judge, LabelChain, Narrowed};
use klatch::gateway::{
    BelowCeiling, ClaudeConfig, ClaudeJudge, DEFAULT_CONFIDENCE_BAND, ProcessRunner, RealRunner,
    drain, rules_judge,
};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Consecutive hosted failures (a defer or a panic) before the breaker stops calling it.
const BREAKER_LIMIT: usize = 3;
/// The cap per hosted call, in dollars.
const HOSTED_BUDGET_USD: f64 = 0.25;
/// How long one hosted call may run.
const HOSTED_TIMEOUT_SECS: u64 = 120;

#[derive(Subcommand)]
pub enum JudgeCmd {
    /// Run every queued item past the judge once and print what happened
    Run {
        /// Do one pass and exit instead of looping
        #[arg(long)]
        once: bool,
        /// Seconds to wait between passes of the resident loop
        #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u64).range(1..))]
        interval_secs: u64,
        /// Stop after this many passes (for tests and supervised runs); without it the loop runs until killed
        #[arg(long, hide = true)]
        max_passes: Option<u64>,
        /// Opt in to a hosted judge: a Claude model (`sonnet`, `opus`, `haiku` or a model id) reached through your own
        /// `claude` binary. OFF unless given, and needs --hosted-ceiling. Enabling it SENDS ROOM TEXT to an outside
        /// service. For our own rooms and evaluation; not a product default.
        #[arg(long, value_name = "MODEL", requires = "hosted_ceiling")]
        hosted_judge: Option<String>,
        /// With --hosted-judge: only text posted strictly below this label is ever sent (anything at or above it, or
        /// with an unknown label, stays queued). There is no default.
        #[arg(long, value_name = "LABEL", requires = "hosted_judge")]
        hosted_ceiling: Option<String>,
        /// With --hosted-judge: a hosted verdict with less confidence than this (0 to 1) is not applied; it goes to the
        /// reviewer as a defer and the audit records it. Default 0.7.
        #[arg(long, value_name = "0..1", requires = "hosted_judge", value_parser = parse_band)]
        hosted_confidence_band: Option<f64>,
    },
}

fn parse_band(s: &str) -> Result<f64, String> {
    s.parse::<f64>()
        .ok()
        .filter(|b| (0.0..=1.0).contains(b))
        .ok_or_else(|| "the confidence band must be a number from 0 to 1".to_string())
}

fn refusal(holds_a_bundle: bool, agent_above: bool) -> Option<&'static str> {
    if holds_a_bundle {
        return Some("a worker cannot run the judge");
    }
    if agent_above {
        return Some(
            "the judge cannot be run from inside a coding-agent session; run this from a launcher of your own",
        );
    }
    None
}

/// One pass: open the store, run the queue past the judge, then apply recorded verdicts with the floors checked
/// again. The store is opened afresh so a long loop never holds a stale handle.
fn pass(hosted: Option<&dyn Judge>) -> Result<()> {
    let store = custody::open_store(&db_path()?)?;
    let rules = rules_judge(&store)?;
    // the rules run first and a rules deny stands; the hosted judge only narrows what they leave
    let out = match hosted {
        Some(h) => drain(&store, &Narrowed::new(rules, h), now())?,
        None => drain(&store, &rules, now())?,
    };
    let gateway = Gateway::new(store, CliClock, AllowAll);
    let released = gateway.release_crossings(&Principal::unattributed("judge"))?;
    println!(
        "judged {} item(s): {} allowed, {} redacted, {} denied; {} left queued",
        out.decided(),
        out.allowed,
        out.redacted,
        out.denied,
        out.deferred
    );
    println!("applied {} crossing verdict(s)", released.len());
    Ok(())
}

/// The hosted stack: the ceiling gate outside the breaker (so traffic above the ceiling never counts as a failure),
/// the breaker outside the transport (so a failing service defers and is not hammered). Built once per loop so the
/// breaker keeps its count across passes.
fn hosted_stack<R: ProcessRunner>(
    config: ClaudeConfig,
    band: f64,
    runner: R,
    workdir: &Path,
) -> Result<BelowCeiling<Breaker<ClaudeJudge<R>>>, String> {
    let ceiling = config.ceiling.clone();
    let judge =
        ClaudeJudge::new(config, runner, workdir, &|k| std::env::var(k).ok())?.with_band(band)?;
    Ok(BelowCeiling::new(
        Breaker::new(judge, BREAKER_LIMIT),
        LabelChain::default(),
        ceiling,
    ))
}

/// A private working directory for the hosted child, removed when dropped.
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir =
            std::env::temp_dir().join(format!("klatch-judge-run-{}-{nanos}", std::process::id()));
        std::fs::create_dir(&dir).context("cannot create the hosted judge's working directory")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(Scratch(dir))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn run(cmd: JudgeCmd) -> Result<()> {
    let JudgeCmd::Run {
        once,
        interval_secs,
        max_passes,
        hosted_judge,
        hosted_ceiling,
        hosted_confidence_band,
    } = cmd;
    if let Some(why) = refusal(
        std::env::var_os("KLATCH_IDENTITY_FILE").is_some(),
        discover_session().is_ok(),
    ) {
        bail!("{why}");
    }
    let limit = if once { Some(1) } else { max_passes };
    let scratch;
    let hosted = match (hosted_judge, hosted_ceiling) {
        (Some(model), Some(ceiling)) => {
            scratch = Scratch::new()?;
            let config = ClaudeConfig {
                model,
                program: "claude".into(),
                budget_usd: HOSTED_BUDGET_USD,
                ceiling: ceiling.clone(),
            };
            let runner = RealRunner {
                timeout: Duration::from_secs(HOSTED_TIMEOUT_SECS),
            };
            let band = hosted_confidence_band.unwrap_or(DEFAULT_CONFIDENCE_BAND);
            let stack =
                hosted_stack(config, band, runner, &scratch.0).map_err(anyhow::Error::msg)?;
            eprintln!(
                "hosted judge ON: room text posted strictly below the label {ceiling:?} is sent to an outside service \
                 through your claude binary; the scrub list and anything at or above that label never is"
            );
            Some(stack)
        }
        _ => None,
    };
    let mut done = 0u64;
    loop {
        // a failed pass (store busy, a bad policy row) is reported and retried; it never decides anything
        if let Err(e) = pass(hosted.as_ref().map(|h| h as &dyn Judge)) {
            if limit == Some(1) {
                return Err(e);
            }
            eprintln!("judge pass failed: {e}");
        }
        done += 1;
        if limit.is_some_and(|l| done >= l) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(interval_secs));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_worker_or_an_agent_session_is_refused() {
        assert!(refusal(false, false).is_none());
        assert!(refusal(true, false).unwrap().contains("worker"));
        assert!(refusal(false, true).unwrap().contains("coding-agent"));
    }

    #[derive(clap::Parser)]
    struct Probe {
        #[command(subcommand)]
        cmd: JudgeCmd,
    }

    fn parses(args: &[&str]) -> bool {
        use clap::Parser;
        let mut v = vec!["probe", "run"];
        v.extend_from_slice(args);
        Probe::try_parse_from(v).is_ok()
    }

    #[test]
    fn the_hosted_judge_is_off_unless_both_flags_are_given() {
        assert!(parses(&[]));
        assert!(parses(&[
            "--hosted-judge",
            "sonnet",
            "--hosted-ceiling",
            "private"
        ]));
        assert!(!parses(&["--hosted-judge", "sonnet"]));
        assert!(!parses(&["--hosted-ceiling", "private"]));
    }

    #[test]
    fn the_confidence_band_needs_the_hosted_judge_and_stays_between_zero_and_one() {
        let hosted = ["--hosted-judge", "sonnet", "--hosted-ceiling", "private"];
        let with = |band: &'static str| [&hosted[..], &["--hosted-confidence-band", band]].concat();
        assert!(parses(&with("0.8")));
        assert!(parses(&with("0")));
        assert!(!parses(&with("1.5")));
        assert!(!parses(&with("-0.1")));
        assert!(!parses(&with("high")));
        assert!(!parses(&["--hosted-confidence-band", "0.8"]));
    }

    struct Down;
    impl ProcessRunner for Down {
        fn run(
            &self,
            _: &klatch::gateway::Invocation,
        ) -> Result<klatch::gateway::ProcessOutput, String> {
            Err("fake transport: down".into())
        }
    }

    #[test]
    fn the_hosted_stack_defers_on_failure_and_rejects_an_unknown_ceiling() {
        let scratch = Scratch::new().unwrap();
        let dir = scratch.0.clone();
        let cfg = |ceiling: &str| ClaudeConfig {
            model: "sonnet".into(),
            program: "claude".into(),
            budget_usd: 0.25,
            ceiling: ceiling.into(),
        };
        assert!(hosted_stack(cfg("nope"), 0.7, Down, &dir).is_err());
        let stack = hosted_stack(cfg("private"), 0.7, Down, &dir).unwrap();
        let item = klatch::domain::ReviewItem {
            id: 1,
            room: "r".into(),
            seq: 1,
            to_label: "public".into(),
            from_label: "public".into(),
            reason: "x".into(),
            text: "t".into(),
        };
        assert_eq!(stack.judge(&item), klatch::domain::Verdict::Defer);
    }
}
