//! `klatch judge run` (#48, #517): drain the review queue through the configured judge, once with `--once` or in a
//! resident loop. The judge is the local rules judge (credential scanner and scrub list): it denies a match and
//! defers everything else, so clean text stays queued for the reviewer. No hosted judge is built here (#53 stays
//! off by default and nothing in this command sends text to a network service). Like the other launcher commands it is refused for a worker and from inside a coding-agent
//! session, because a verdict is policy and an agent must not rule on its own messages.

use super::*;
use clap::Subcommand;
use klatch::gateway::{drain, rules_judge};
use std::time::Duration;

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
    },
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
fn pass() -> Result<()> {
    let store = custody::open_store(&db_path()?)?;
    let judge = rules_judge(&store)?;
    let out = drain(&store, &judge, now())?;
    let gateway = Gateway::new(store, CliClock, AllowAll);
    let released = gateway.release_crossings()?;
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

pub fn run(cmd: JudgeCmd) -> Result<()> {
    let JudgeCmd::Run {
        once,
        interval_secs,
        max_passes,
    } = cmd;
    if let Some(why) = refusal(
        std::env::var_os("KLATCH_IDENTITY_FILE").is_some(),
        discover_session().is_ok(),
    ) {
        bail!("{why}");
    }
    let limit = if once { Some(1) } else { max_passes };
    let mut done = 0u64;
    loop {
        // a failed pass (store busy, a bad policy row) is reported and retried; it never decides anything
        if let Err(e) = pass() {
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
}
