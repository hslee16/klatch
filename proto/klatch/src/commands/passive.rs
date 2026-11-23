//! The hook side of passive claims (#1401): a PostToolUse input that edited or wrote a file becomes a claim on its
//! repo-relative path, a refusal becomes one line for the model, and the Stop and SessionEnd hooks release what was
//! taken. Tool parsing and the repo-relative path are the machine checkpoint's (`checkpoint::tool_trace`): the command
//! line, the file content and the tool output are read there and dropped. Nothing here can fail the tool call: every
//! error is swallowed (the gateway counts it) and nothing is printed unless another member holds the path.

use super::*;
use klatch::domain::{PassiveOutcome, refusal_line};

/// Claim each path the tool call edited or wrote, when the hook carries the consent; the line to print when another
/// member holds one of them, else none. More than one refused path is told as the first line and a count.
pub(super) fn claim_edits(
    t: &dyn Transport,
    input: Option<&serde_json::Value>,
    consent: bool,
) -> Option<String> {
    if !consent {
        return None;
    }
    let (class, paths) = input.and_then(super::checkpoint::tool_trace)?;
    if !class.keeps_paths() {
        return None;
    }
    let mut lines = Vec::new();
    for path in &paths {
        if let Ok(PassiveOutcome::Refused(r)) = t.passive_claim(path) {
            lines.push(refusal_line(path, &r));
        }
    }
    let mut lines = lines.into_iter();
    let first = lines.next()?;
    let more = lines.count();
    Some(if more == 0 {
        first
    } else {
        format!(
            "{first} ({more} more of the paths you edited are held by others too; `klatch claims` lists them)"
        )
    })
}

/// Release the passive claims at the end of a turn or a session, when the hook carries the consent. A failure is
/// swallowed: the lease frees them anyway.
pub(super) fn release(t: &dyn Transport, consent: bool) {
    if consent {
        let _ = t.release_passive();
    }
}
