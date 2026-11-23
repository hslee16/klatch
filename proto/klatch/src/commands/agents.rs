//! Hooks and the agent roles.

use super::machine_status::background_counts;
use super::*;

/// Hooks and the agent roles; every other command was handled before dispatch.
pub(super) fn run_agents(ctx: &Ctx, cmd: AgentsCmd) -> Result<()> {
    let t = ctx.t;
    match cmd {
        AgentsCmd::Hooks { of } => hooks(ctx, of)?,
        AgentsCmd::Router {
            room,
            rules,
            once,
            interval,
        } => router(ctx, (&room, &rules), (once, interval))?,
        AgentsCmd::Mediator {
            room,
            once,
            interval,
        } => mediator(ctx, &room, (once, interval))?,
        AgentsCmd::Hook {
            event,
            board,
            no_join,
            checkpoint,
            machine_status,
            passive_claims,
        } => hook(
            ctx,
            &HookArgs {
                event,
                board,
                no_join,
                checkpoint,
                machine_status,
                passive_claims,
            },
        )?,
        AgentsCmd::Act { cmd } => roles::act(t, cmd)?,
        AgentsCmd::Review { cmd } => roles::review(t, cmd)?,
    }
    Ok(())
}

/// `klatch router` (#189): passes of the reference router as the caller's own member.
pub(super) fn router(
    ctx: &Ctx,
    (room, rules): (&str, &[String]),
    (once, interval): (bool, u64),
) -> Result<()> {
    let rules = rules
        .iter()
        .map(|r| klatch::domain::Rule::parse(r).map_err(|e| anyhow::anyhow!(e)))
        .collect::<Result<Vec<_>>>()?;
    let mut router = klatch::router::Router::new(ctx.me, room, rules);
    let mut now = ctx.now;
    loop {
        for event in router.pass(ctx.t, now)? {
            println!("{event}");
        }
        if once {
            return Ok(());
        }
        // a pass already refreshed presence (it reads the room); between passes the router only waits
        std::thread::sleep(std::time::Duration::from_secs(interval.max(1)));
        now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(now, |d| i64::try_from(d.as_secs()).unwrap_or(now));
    }
}

/// `klatch mediator` (#271): passes of the reference mediator as the caller's own member.
pub(super) fn mediator(ctx: &Ctx, room: &str, (once, interval): (bool, u64)) -> Result<()> {
    let mut mediator = klatch::mediator::Mediator::new(ctx.me, room);
    let mut now = ctx.now;
    loop {
        for event in mediator.pass(ctx.t, now)? {
            println!("{event}");
        }
        if once {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_secs(interval.max(1)));
        now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(now, |d| i64::try_from(d.as_secs()).unwrap_or(now));
    }
}

pub(super) fn hooks(ctx: &Ctx, of: Option<String>) -> Result<()> {
    let who = of.unwrap_or_else(|| ctx.me.to_string());
    let runs = ctx.t.hook_runs(&who)?;
    if runs.is_empty() {
        println!("no hook has run for {who}");
    }
    for r in runs {
        println!(
            "{}\tlast ran {}s ago\truns {}\tprinted {} bytes\t{} in all (largest {})",
            r.event,
            klatch::domain::secs_between(ctx.now, r.last_ts),
            r.runs,
            r.last_bytes,
            r.total_bytes,
            r.max_bytes
        );
    }
    Ok(())
}

/// The caller's own refusals, newest first: time, room, operation and rule. Never the text.
pub(super) fn my_refusals(ctx: &Ctx, limit: usize) -> Result<()> {
    let list = ctx.t.my_refusals(limit)?;
    if list.is_empty() {
        println!("no refusals recorded for {}", ctx.me);
    }
    for r in list {
        println!(
            "{}s ago\t{}\t{}\t{}",
            klatch::domain::secs_between(ctx.now, r.ts),
            flat(&r.room),
            r.operation,
            r.rule
        );
    }
    Ok(())
}

/// What a hook was started with.
pub(super) struct HookArgs {
    pub(super) event: Option<String>,
    pub(super) board: Option<String>,
    pub(super) no_join: bool,
    /// The machine checkpoint consent (#1408): the hook command carries `--checkpoint`.
    pub(super) checkpoint: bool,
    /// The machine status consent (#1403): the hook command carries `--machine-status`.
    pub(super) machine_status: bool,
    /// The passive claims consent (#1401): the hook command carries `--passive-claims`.
    pub(super) passive_claims: bool,
}

impl HookArgs {
    fn is(&self, event: &str) -> bool {
        self.event.as_deref() == Some(event)
    }
}

/// A session that ends is marked done in every room it joined, so it leaves the board at once instead of aging out
/// (DESIGN section 25.13). It prints nothing: the session is over.
pub(super) fn session_end(t: &dyn Transport, ended: Option<&str>, passive: bool) -> Result<()> {
    // the final machine checkpoint (#1408) first: it names the claims the release below frees
    super::checkpoint::end_checkpoint(t, ended);
    // the passive claims (#1401) before the rest, so they are counted as passive releases
    super::passive::release(t, passive);
    // Only the rooms are needed: a digest would mark unread messages as shown, and a resumed session would never
    // be shown them (#30).
    // a member that has ended cannot be working: free what it held (#277). First, so a credential that idled out
    // before the hook (which refuses `rooms`, #835) still frees the claims.
    let _ = t.release_all();
    let rooms = t.rooms()?;
    for room in &rooms {
        let _ = t.set_status(room, klatch::domain::SESSION_END_STATUS);
    }
    let _ = t.record_hook_run("SessionEnd", 0);
    // a worker that holds a credential loses it with its session (a launcher can issue another)
    let _ = t.end_credential();
    Ok(())
}

/// Claude Code puts `agent_id` in a hook's input only when the tool call was made inside a subagent.
pub(super) fn inside_subagent(input: &serde_json::Value) -> bool {
    input
        .get("agent_id")
        .and_then(|v| v.as_str())
        .is_some_and(|id| !id.is_empty())
}

/// Remember the permission mode this session reports (#553): it is the only place Claude Code says it, and `klatch
/// doctor` needs it to warn that a bypassing member's socket nudge is held. A subagent's tool call carries the
/// subagent's mode, which says nothing about this session, and a failure to record must not stop the hook.
pub(super) fn note_permission_mode(ctx: &Ctx) {
    let Some(input) = ctx.stdin.filter(|i| !inside_subagent(i)) else {
        return;
    };
    if let Some(mode) = input.get("permission_mode").and_then(|m| m.as_str()) {
        let _ = ctx.t.note_member_mode(mode);
    }
}

pub(super) fn hook(ctx: &Ctx, args: &HookArgs) -> Result<()> {
    let t = ctx.t;
    if args.is("SessionEnd") {
        return session_end(
            t,
            super::checkpoint::end_reason(ctx.stdin, args.checkpoint),
            args.passive_claims,
        );
    }
    note_permission_mode(ctx);
    if args.is("PreToolUse") {
        return pre_tool_use(ctx);
    }
    // A tool call made by a subagent fires this hook as the parent session, whose member did not make the call. The
    // subagent speaks under its own typed name, so showing it mail here echoes its own posts and claims back and eats
    // the parent's unread mail (#493). Stay silent and leave every cursor where it is.
    if ctx.stdin.is_some_and(inside_subagent) {
        return Ok(());
    }
    let passive = note_activity(ctx, args);
    if args.is("Stop") {
        stop_hook(ctx, args);
        return Ok(());
    }
    // Any other hook is a sign of life: it ends a park.
    let _ = t.sign_of_life();
    let (mut text, notice) = hook_text(ctx, args)?;
    if let Some(line) = passive {
        append_line(&mut text, &line);
    }
    print_hook(ctx, args, &text, notice.as_deref());
    Ok(())
}

fn append_line(text: &mut String, line: &str) {
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(line);
}

/// Record that this hook ran (the size it printed, never the content), so a user can see the hooks are alive for a
/// session, then print it. A failure to record must not stop the hook delivering.
fn print_hook(ctx: &Ctx, args: &HookArgs, text: &str, notice: Option<&str>) {
    let _ = ctx
        .t
        .record_hook_run(args.event.as_deref().unwrap_or("hook"), text.len());
    match &args.event {
        Some(e) => print!("{}", hook_output_json(e, text, notice)),
        None => print!("{text}"),
    }
}

/// The bookkeeping a hook does before it prints: which member this session is, the tool call and the machine status.
/// Returns the line about a file this call claimed passively, if any.
fn note_activity(ctx: &Ctx, args: &HookArgs) -> Option<String> {
    let t = ctx.t;
    // Remember which member this session is, so a hook that runs after the harness dropped its own record (the
    // SessionEnd hook) can still find it.
    if let Some(sid) = ctx.stdin_session {
        let _ = t.note_session(sid);
    }
    // The end of a turn records what still runs in the background (a declared long wait) and prints nothing: the
    // agent did not have to announce it and has nothing to read (DESIGN section 32 item 45).
    super::checkpoint::note_tool_call(t, ctx.stdin, args.checkpoint && args.is("PostToolUse"));
    super::machine_status::on_tool_use(t, args.machine_status && args.is("PostToolUse"));
    // a member that edits a file claims it, when the hook carries the consent (#1401); a refusal is one line below
    super::passive::claim_edits(t, ctx.stdin, args.passive_claims && args.is("PostToolUse"))
}

/// The end of a turn: what still runs in the background is recorded and released claims are freed; nothing is printed.
fn stop_hook(ctx: &Ctx, args: &HookArgs) {
    let t = ctx.t;
    let (tasks, crons) = ctx.stdin.map_or((0, 0), background_counts);
    let _ = t.record_stop(tasks, crons);
    super::machine_status::on_stop(t, args.machine_status, tasks + crons > 0);
    super::checkpoint::stop_checkpoint(t, args.checkpoint);
    super::passive::release(t, args.passive_claims);
    let _ = t.record_hook_run("Stop", 0);
}

/// The text a hook prints and the notice for the human: the session start, or the digest of what is new.
fn hook_text(ctx: &Ctx, args: &HookArgs) -> Result<(String, Option<String>)> {
    let t = ctx.t;
    if args.is("SessionStart") {
        return session_start(ctx, args);
    }
    Ok((
        digest_text(
            &t.digest_with(false)?,
            session_level(t, ctx.me),
            args.is("UserPromptSubmit"),
        ),
        None,
    ))
}

#[cfg(test)]
mod stop_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_stop_input_is_counted_whatever_shape_the_lists_have() {
        let lists = json!({"background_tasks": [{"id": "a"}, {"id": "b"}], "session_crons": [1]});
        assert_eq!(background_counts(&lists), (2, 1));
        let numbers = json!({"background_tasks": 3, "session_crons": 0});
        assert_eq!(background_counts(&numbers), (3, 0));
        assert_eq!(background_counts(&json!({"background_tasks": "x"})), (0, 0));
        assert_eq!(background_counts(&json!({})), (0, 0));
    }
}
