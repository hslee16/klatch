//! `klatch init --codex` (#782): install klatch's synchronous hooks into Codex's `hooks.json`, with the same consent and
//! diff as the Claude Code installer. The wake hook is never written (Codex skips async hooks, RESEARCH-NOTES 49.7);
//! the trust record is never written either (klatch cannot, and never passes `--dangerously-bypass-hook-trust`), so the
//! run ends by telling the human to review the hooks in `/hooks`.

use crate::init::{self, Io, Options, Places, Plan};
use anyhow::{Context, bail};

/// The flag that makes the member come from the hook's own session id and working directory (#341).
const MODE: &str = "--codex";

/// What `init --codex` says after it has written, or found nothing to write.
const TRUST_STEP: &str = "Codex runs a hook only after you review it: open Codex and trust these in /hooks. klatch cannot write that record and never passes --dangerously-bypass-hook-trust, so until you do, Codex skips them (silently in `codex exec`).
The wake watcher is not installed for Codex: Codex skips async hooks, so only the synchronous hooks above exist.
";

/// Options that only mean something for Claude Code's settings.
fn refuse_claude_only(options: &Options) -> anyhow::Result<()> {
    let claude_only = [
        (options.scope.is_some(), "--scope"),
        (options.no_session_end, "--no-session-end"),
        (options.enforce != init::Switch::Keep, "--enforce"),
        (options.wake != init::Switch::Keep, "--wake"),
        (options.checkpoint != init::Switch::Keep, "--checkpoint"),
        (
            options.passive_claims != init::Switch::Keep,
            "--passive-claims",
        ),
        (
            options.machine_status != init::Switch::Keep,
            "--machine-status",
        ),
        (options.stop_warn != init::Switch::Keep, "--stop-warn"),
    ];
    if let Some((_, flag)) = claude_only.iter().find(|(set, _)| *set) {
        bail!(
            "{flag} is for Claude Code's settings; with --codex klatch writes only Codex's hooks.json (the synchronous hooks, never the wake or claim hooks)"
        );
    }
    Ok(())
}

/// Run `init --codex`: ask (unless `--yes`), show the diff of `<CODEX_HOME>/hooks.json`, write on a yes.
pub fn run(options: &Options, io: &mut Io, places: &Places) -> anyhow::Result<()> {
    refuse_claude_only(options)?;
    let path = places
        .codex_home
        .context("neither CODEX_HOME nor HOME is set, so the Codex directory cannot be found")?
        .join("hooks.json");
    if options.uninstall {
        return init::remove_from(options.yes, io, vec![("codex", path)]);
    }
    let bin = places
        .bin
        .to_str()
        .context("the path of this binary is not valid text")?;
    let board = io.choose_board(options)?;
    let join = io.on_unless(
        options.no_lobby,
        options.yes,
        "Join every new Codex session to the default lobby automatically?",
    )?;
    let existing = init::read_settings(&path)?;
    let merged = init::merge_commands(existing.as_ref(), |event| {
        init::command_with(bin, MODE, event, (board.as_deref(), join))
    })
    .map_err(|e| anyhow::anyhow!("{}: {e}; nothing was written", path.display()))?;
    init::apply(
        Plan {
            label: "codex",
            path,
            existing,
            merged,
        },
        options.yes,
        io,
    )?;
    write!(io.out, "{TRUST_STEP}")?;
    Ok(())
}
