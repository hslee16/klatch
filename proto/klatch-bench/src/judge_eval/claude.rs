//! A plain LLM judge: one headless `claude -p` call per item (#914), the quarantined-reviewer shape of DESIGN D4
//! (one candidate message plus the rules, no tools, a fixed schema back).
//!
//! The session is isolated the way `proto/scripts/team` isolates its workers, and further: no setting source is
//! loaded (`--setting-sources ""`, so none of the user's, the project's or the local settings, and none of their
//! hooks), the only settings file is a throwaway one that sets `disableAllHooks`, no tools, no MCP servers, no slash
//! commands, no session saved, the default system prompt replaced, a dollar cap per call, a working directory that is
//! a fresh temporary directory, and an environment cleared down to an allow-list that has no `KLATCH_*` or `CLAUDE*`
//! variable. The message text goes on stdin inside a fence it cannot close and never on the command line. The
//! process is behind [`ProcessRunner`] so tests use a fake and no model.
//!
//! The transport, the prompt, the isolation flags and the reply parser live in `klatch::gateway` (#960, #1014) and
//! are shared with the opt-in hosted judge of `klatch judge run`. This module adds what only the bench needs: the
//! scrub list in the system prompt, the confidence, token and cost accounting, and a runner that starts the binary
//! the `--claude` option names.

use crate::judge_eval::corpus::Truth;
use crate::judge_eval::judges::{EvalJudge, Judged};
use klatch::domain::{Judge, ReviewItem, Verdict};
use klatch::gateway::{self, Pick, Session};
use std::path::Path;

pub use klatch::gateway::{Invocation, ProcessOutput, ProcessRunner, fence_for};

/// The settings.
#[derive(Debug, Clone)]
pub struct ClaudeConfig {
    /// `opus`, `sonnet`, `haiku` or a full model id.
    pub model: String,
    /// The `claude` binary.
    pub program: String,
    /// The cap per call (`--max-budget-usd`).
    pub budget_usd: f64,
    /// The scrub-list terms, given to the model as part of the rules.
    pub scrub_terms: Vec<String>,
    /// Only text posted strictly below this label is sent (the hosted judge's gate, #53).
    pub ceiling: String,
}

fn truth(p: Pick) -> Truth {
    match p {
        Pick::Allow => Truth::Allow,
        Pick::Redact => Truth::Redact,
        Pick::Deny => Truth::Deny,
    }
}

/// What a reply said, checked.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub pick: Truth,
    pub confidence: f64,
    pub redacted_text: Option<String>,
    pub reason: Option<String>,
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
    pub cost_usd: Option<f64>,
}

/// Read `claude -p --output-format json` output. Errors never quote the reply, which can echo the message.
pub fn parse_output(stdout: &str) -> Result<Parsed, String> {
    let p = gateway::parse_reply(stdout)?;
    Ok(Parsed {
        pick: truth(p.pick),
        confidence: p.confidence,
        redacted_text: p.redacted_text,
        reason: p.reason,
        tokens_in: p.tokens_in,
        tokens_out: p.tokens_out,
        cost_usd: p.cost_usd,
    })
}

/// The judge. It is a production [`Judge`] as well as an [`EvalJudge`].
pub struct ClaudeJudge<R: ProcessRunner> {
    model: String,
    session: Session,
    runner: R,
}

impl<R: ProcessRunner> ClaudeJudge<R> {
    /// Check the settings and write the throwaway settings file into `workdir` (which must exist and should be a
    /// fresh temporary directory). `env` looks up the parent's variables; only the shared allow-list is copied.
    pub fn new(
        config: ClaudeConfig,
        runner: R,
        workdir: &Path,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        let model = config.model.clone();
        let shared = gateway::ClaudeConfig {
            model: config.model,
            program: config.program,
            budget_usd: config.budget_usd,
            ceiling: config.ceiling,
        };
        let session = Session::new(shared, Some(&config.scrub_terms), workdir, env)?;
        Ok(ClaudeJudge {
            model,
            session,
            runner,
        })
    }

    pub fn invocation(&self, item: &ReviewItem) -> Invocation {
        self.session.invocation(item)
    }

    fn call(&self, item: &ReviewItem) -> Judged {
        if !self.session.admits(item) {
            return Judged::of(Verdict::Defer);
        }
        let p = match self.session.run(&self.runner, item) {
            Ok(p) => p,
            Err(e) => return Judged::failed(e),
        };
        let verdict = match p.pick {
            Pick::Allow => Verdict::Allow,
            Pick::Redact => Verdict::Redact(p.redacted_text.clone().unwrap_or_default()),
            Pick::Deny => Verdict::Deny(
                p.reason
                    .clone()
                    .unwrap_or_else(|| "the claude judge denied it".into()),
            ),
        };
        Judged {
            verdict,
            raw: Some((truth(p.pick), p.confidence)),
            probability: None,
            stage: None,
            tokens_in: p.tokens_in,
            tokens_out: p.tokens_out,
            cost_usd: p.cost_usd,
            error: None,
        }
    }
}

impl<R: ProcessRunner> Judge for ClaudeJudge<R> {
    fn judge(&self, item: &ReviewItem) -> Verdict {
        self.call(item).verdict
    }
}

impl<R: ProcessRunner> EvalJudge for ClaudeJudge<R> {
    fn name(&self) -> String {
        format!("claude:{}", self.model)
    }

    fn evaluate(&self, item: &ReviewItem) -> Judged {
        self.call(item)
    }
}

/// Runs the real process through the shared supervisor: the binary the invocation names (the `--claude` option),
/// an empty environment plus the given one, killed after `timeout`.
pub struct RealRunner {
    pub timeout: std::time::Duration,
}

impl ProcessRunner for RealRunner {
    fn run(&self, inv: &Invocation) -> Result<ProcessOutput, String> {
        gateway::run_command(std::process::Command::new(&inv.program), inv, self.timeout)
    }
}

#[cfg(test)]
#[path = "claude_tests.rs"]
mod tests;
