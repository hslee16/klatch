//! The opt-in PreToolUse overlap step (#1402, #1458): before an Edit, Write, MultiEdit or NotebookEdit, say in one
//! bounded line when another member holds a live claim on the path (explicit or passive), and, with the claims consent,
//! take the passive claim for the editor so even its first edit of a path is covered at edit time.
//!
//! It is a warning and never a block: `init --enforce` is the separate blocking hook, which this neither replaces nor
//! weakens. It prints nothing when there is nothing to say, never reads a message (so it cannot consume one), never
//! prints a claim's reason, and reads only claims in rooms the member can read (the guarded `claims` call). A store it
//! cannot read fails open: `main` prints the one visible line it prints for every failed PreToolUse hook. No network.

use super::*;
use klatch::domain::{
    EDIT_TOOLS, PASSIVE_REASON, PassiveOutcome, pre_edit_refusal_line, pre_edit_warning,
};

/// Every claim another member holds in every room this member can read, at its real place: a passive claim names a
/// repo-relative path, so it is taken from the repository root, an explicit one from the working directory (as the
/// blocking hook reads it); each claim is also given where its path really is, so a symlink cannot hide it.
fn others_claims(t: &dyn Transport, me: &str, cwd: &str) -> Result<Vec<Held>> {
    let root = super::checkpoint::repo_root(cwd);
    let mut held = Vec::new();
    for room in t.rooms()? {
        // a room this member may not read has no claims it may be told of
        let Ok(claims) = t.claims(&room) else {
            continue;
        };
        for claim in claims.into_iter().filter(|c| c.owner != me) {
            let base = if claim.reason == PASSIVE_REASON {
                root.as_str()
            } else {
                cwd
            };
            let path = klatch::domain::clean_path(&claim.path, base);
            let real = real_path(&path, base);
            for path in std::iter::once(path).chain(real) {
                held.push(Held {
                    room: room.clone(),
                    claim: Claim {
                        path,
                        ..claim.clone()
                    },
                });
            }
        }
    }
    Ok(held)
}

/// The spellings of each path the call names: as written (cleaned) and where it really is (symlinks, a file that does
/// not exist yet kept under its nearest real directory).
fn spellings(call: &ToolInput) -> Vec<Vec<String>> {
    call.targets
        .iter()
        .map(|t| {
            let mut all = vec![klatch::domain::clean_path(t, &call.cwd)];
            all.extend(real_path(t, &call.cwd));
            all
        })
        .collect()
}

/// `PreToolUse` with `--pre-edit-warn` (and `--pre-edit-claims`): the line to tell the member, if any.
pub(super) fn pre_edit(ctx: &Ctx, claims: bool) -> Result<()> {
    let Some(input) = ctx.stdin else {
        return Ok(());
    };
    // a subagent's tool call fires this hook as its parent, whose member did not make it: stay out of it
    if inside_subagent(input) {
        return Ok(());
    }
    let Some(call) = tool_input(input) else {
        return Ok(());
    };
    if !EDIT_TOOLS.contains(&call.tool.as_str()) || call.targets.is_empty() {
        return Ok(());
    }
    let t = ctx.t;
    let held = others_claims(t, ctx.me, &call.cwd)?;
    let scope = t.scope()?.unwrap_or_default();
    let who = Asker {
        member: ctx.me,
        scope: &scope,
        now: ctx.now,
    };
    // Nothing is held by someone else: cover the path before the edit. A warning is never replaced by the claim's
    // outcome and takes no claim (a refused claim would only record a wait), so the claim is tried only without one.
    let line = pre_edit_warning(&spellings(&call), &held, &who)
        .or_else(|| claims.then(|| claim_first(t, input)).flatten());
    if let Some(line) = line {
        // the line goes into the context, so it counts toward the session's bytes (#1406)
        let _ = t.record_hook_run("PreToolUse", line.len());
        print!("{}", hook_output_json("PreToolUse", &line, None));
    }
    Ok(())
}

/// Take the passive claim of each path the call edits (the repo-relative paths the PostToolUse step claims, by the same
/// rules); the line for the first one another member holds. Any failure is swallowed: the tool call goes on.
fn claim_first(t: &dyn Transport, input: &serde_json::Value) -> Option<String> {
    let (class, paths) = super::checkpoint::tool_trace(input)?;
    if !class.keeps_paths() {
        return None;
    }
    paths.iter().find_map(|path| match t.passive_claim(path) {
        Ok(PassiveOutcome::Refused(r)) => Some(pre_edit_refusal_line(path, &r)),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use klatch::domain::Principal;
    use klatch::gateway::{AllowAll, Clock, Direct, Gateway};
    use klatch::store::SqliteStore;
    use std::rc::Rc;

    struct Fixed;
    impl Clock for Fixed {
        fn now(&self) -> i64 {
            1000
        }
    }

    #[test]
    fn a_store_or_gateway_error_inside_the_step_is_an_error_for_main_to_show_and_never_a_panic_or_a_block()
     {
        let g = Rc::new(Gateway::new(
            SqliteStore::open_in_memory().unwrap(),
            Fixed,
            AllowAll,
        ));
        // a member name the gateway refuses: its `rooms` call fails inside the step
        let t = Direct::new(g, Principal::unattributed("not a valid name!"));
        let input = serde_json::json!({"tool_name": "Edit", "cwd": "/w",
            "tool_input": {"file_path": "/w/src/a.rs"}});
        let ctx = Ctx {
            t: &t,
            me: "not a valid name!",
            now: 1000,
            stdin_session: None,
            discovered: false,
            stdin: Some(&input),
        };
        assert!(pre_edit(&ctx, true).is_err());
    }
}
