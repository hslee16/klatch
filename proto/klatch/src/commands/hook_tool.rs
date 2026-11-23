//! The `PreToolUse` part of a hook (split from `agents.rs`, #1448; no behaviour change).

use super::*;

/// `PreToolUse`: judge the call against the claims other members hold (#33). A blocked edit prints the reason on
/// standard error and exits 2, which stops the call even under bypassPermissions; a warning goes into the context;
/// anything unreadable allows the call (fail open, with the visible line `main` prints for a failed hook).
pub(super) fn pre_tool_use(ctx: &Ctx) -> Result<()> {
    let Some(call) = ctx.stdin.and_then(tool_input) else {
        return Ok(());
    };
    refuse_unnamed_subagent(ctx, &call);
    let t = ctx.t;
    let held = held_claims(t, &call.cwd)?;
    let (targets, command) = judged_words(&call);
    let scope = t.scope()?.unwrap_or_default();
    let verdict = judge_tool(
        &ToolCall {
            tool: &call.tool,
            targets: &targets,
            command: &command,
            cwd: &call.cwd,
        },
        &Asker {
            member: ctx.me,
            scope: &scope,
            now: ctx.now,
        },
        &held,
    );
    match verdict {
        ToolVerdict::Allow => {}
        ToolVerdict::Warn(w) => {
            // a warning goes into the context, so it counts toward the session's bytes (#1406)
            let _ = t.record_hook_run("PreToolUse", w.len());
            print!("{}", hook_output_json("PreToolUse", &w, None));
        }
        ToolVerdict::Block(reason) => {
            eprintln!("{reason}");
            std::process::exit(2);
        }
    }
    Ok(())
}

/// A subagent has no process of its own and no agent id in its environment, so a klatch command it runs would
/// resolve to the parent's member. This hook sees `agent_id`: refuse the command unless it names the subagent (#1152).
fn refuse_unnamed_subagent(ctx: &Ctx, call: &ToolInput) {
    if is_unnamed_klatch_in_subagent(ctx, call) {
        eprintln!(
            "klatch: this command runs inside a subagent and does not name itself, so it would act as the session's \
             member and share its name and cursor. Set KLATCH_NAME to a name for this subagent (for example \
             KLATCH_NAME=<task>-1) in front of the klatch command and run it again."
        );
        std::process::exit(2);
    }
}

fn is_unnamed_klatch_in_subagent(ctx: &Ctx, call: &ToolInput) -> bool {
    call.tool == "Bash"
        && ctx.stdin.is_some_and(inside_subagent)
        && klatch::domain::invokes_klatch_unnamed(&call.command)
}

/// Every claim held in every room this member can read, and each claim again at its real place.
fn held_claims(t: &dyn Transport, cwd: &str) -> Result<Vec<Held>> {
    let mut held = Vec::new();
    for room in t.rooms()? {
        // a room this member may not read has no claims it can be judged by
        if let Ok(claims) = t.claims(&room) {
            held.extend(claims.into_iter().map(|claim| Held {
                room: room.clone(),
                claim,
            }));
        }
    }
    // A symlink must not hide a claimed file: judge the real place of each claim and each target as well.
    let real_claims: Vec<Held> = held
        .iter()
        .filter_map(|h| {
            let path = real_path(&h.claim.path, cwd)?;
            Some(Held {
                room: h.room.clone(),
                claim: Claim {
                    path,
                    ..h.claim.clone()
                },
            })
        })
        .collect();
    held.extend(real_claims);
    Ok(held)
}

/// The targets and the command text to judge: each as given, plus where each path really is.
fn judged_words(call: &ToolInput) -> (Vec<String>, String) {
    let mut targets = call.targets.clone();
    targets.extend(call.targets.iter().filter_map(|p| real_path(p, &call.cwd)));
    // The same for the paths a shell command names: add where each one really is to what is judged.
    let mut command = call.command.clone();
    for real in klatch::domain::command_words(&call.command)
        .filter(|w| w.contains('/'))
        .filter_map(|w| real_path(w, &call.cwd))
    {
        command.push(' ');
        command.push_str(&real);
    }
    (targets, command)
}
