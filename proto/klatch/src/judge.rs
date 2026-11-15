//! `klatch judge run --once` (#48): drain the review queue once through the configured judge. A daemon is later just
//! a loop around this command. No judge is configured today, so this is the `NullJudge`: it decides nothing and the
//! queue only grows. Like the other launcher commands it is refused for a worker and from inside a coding-agent
//! session, because a verdict is policy and an agent must not rule on its own messages.

use super::*;
use clap::Subcommand;
use klatch::domain::NullJudge;
use klatch::gateway::drain;

#[derive(Subcommand)]
pub enum JudgeCmd {
    /// Run every queued item past the judge once and print what happened
    Run {
        /// Required: a resident loop does not exist yet, so say that one pass is what you mean
        #[arg(long)]
        once: bool,
    },
}

fn refusal(holds_a_bundle: bool, agent_above: bool, once: bool) -> Option<&'static str> {
    if holds_a_bundle {
        return Some("a worker cannot run the judge");
    }
    if agent_above {
        return Some(
            "the judge cannot be run from inside a coding-agent session; run this from a launcher of your own",
        );
    }
    if !once {
        return Some("only `klatch judge run --once` exists; a resident judge is not built");
    }
    None
}

pub fn run(cmd: JudgeCmd) -> Result<()> {
    let JudgeCmd::Run { once } = cmd;
    if let Some(why) = refusal(
        std::env::var_os("KLATCH_IDENTITY_FILE").is_some(),
        discover_session().is_ok(),
        once,
    ) {
        bail!("{why}");
    }
    let store = custody::open_store(&db_path()?)?;
    let out = drain(&store, &NullJudge, now())?;
    println!(
        "judged {} item(s): {} allowed, {} redacted, {} denied; {} left queued",
        out.decided(),
        out.allowed,
        out.redacted,
        out.denied,
        out.deferred
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_worker_an_agent_session_or_a_missing_once_is_refused() {
        assert!(refusal(false, false, true).is_none());
        assert!(refusal(true, false, true).unwrap().contains("worker"));
        assert!(refusal(false, true, true).unwrap().contains("coding-agent"));
        assert!(refusal(false, false, false).unwrap().contains("--once"));
    }
}
