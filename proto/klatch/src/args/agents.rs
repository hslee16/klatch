//! Hook and agent role commands.

use super::*;

#[derive(Subcommand)]
pub(crate) enum AgentsCmd {
    /// Call the acts your role allows, as yourself (admin agent; not `klatch admin`, which stays the person's)
    #[command(display_order = 20)]
    Act {
        #[command(subcommand)]
        cmd: roles::ActCmd,
    },
    /// Read the review queue and record verdicts (reviewer only)
    #[command(display_order = 21)]
    Review {
        #[command(subcommand)]
        cmd: roles::ReviewCmd,
    },
    /// Run the reference router as your own member (#189): read the lobby's open asks, claims and presence and invite
    /// the members that hold a claim on an asker's topic into a topic room. Rules only, no model; it needs the
    /// create-room and invite capability like any member, and it decides nothing from message text.
    #[command(display_order = 30)]
    Router {
        /// The room to watch and to post invite pointers in
        #[arg(long, default_value = "lobby")]
        room: String,
        /// An operator rule `claim-path-prefix=member`: invite that member for a topic under the prefix (repeatable)
        #[arg(long = "rule")]
        rules: Vec<String>,
        /// One pass, then exit
        #[arg(long)]
        once: bool,
        /// Seconds between passes
        #[arg(long, default_value_t = 10)]
        interval: u64,
    },
    /// Run the reference mediator as your own member (#271): read the room's stuck-case events (deadlock, stale ask,
    /// nobody capable, ask cycle) and post proposals for them, and for a cycle through asks create a room and invite
    /// the members into it. Rules only, no model. It only proposes: it cancels, releases, grants and clears nothing,
    /// holds no admin act, needs the create-room and invite capability like any member, and decides nothing from
    /// message text
    #[command(display_order = 31)]
    Mediator {
        /// The room to watch and to post proposals in
        #[arg(long, default_value = "lobby")]
        room: String,
        /// One pass, then exit
        #[arg(long)]
        once: bool,
        /// Seconds between passes
        #[arg(long, default_value_t = 10)]
        interval: u64,
    },
    /// Show when each hook event last ran for you (or for another member with --of), without any content
    #[command(display_order = 32)]
    Hooks {
        /// A member name to look at instead of yourself
        #[arg(long)]
        of: Option<String>,
    },
    /// Hook output for UserPromptSubmit/SessionStart: unread messages and claims across joined rooms
    #[command(display_order = 45)]
    Hook {
        /// Print JSON additional context for this Claude Code hook event (needed for PostToolUse, whose plain
        /// stdout does not reach the model). Without it, plain text.
        #[arg(long)]
        event: Option<String>,
        /// At session start also print this room's board (open asks, offers, members) without joining it.
        /// Consent lives here: only a hook command that carries this flag reads a room before joining it.
        #[arg(long)]
        board: Option<String>,
        /// At session start, do not join the default lobby (the default is to join it, as the session's first act)
        #[arg(long)]
        no_join: bool,
        /// Keep a machine checkpoint (#1408): on PostToolUse note the class of each tool call and the repo-relative
        /// path of an edit or write (never a command, content or output); on Stop and SessionEnd write the checkpoint
        /// beside the member's task. Consent lives here: only a hook command that carries this flag records anything
        /// (`klatch init --checkpoint`).
        #[arg(long)]
        checkpoint: bool,
        /// Keep the member's status from activity (#1403): `working (auto)` at the first tool call after idle
        /// (PostToolUse), `idle (auto)` or `parked (auto)` at the end of a turn (Stop). Posts no message, records
        /// no tool, command or content, and never replaces a status the member set. Consent lives here: only a
        /// hook command that carries this flag writes anything (`klatch init --machine-status`).
        #[arg(long)]
        machine_status: bool,
        /// Warn at the end of a turn that still has background tasks or crons running (#1344): print one fixed line
        /// with counts and kinds, never a command or description, and stop nothing. Consent lives here: only a hook
        /// command that carries this flag prints it (`klatch init --stop-warn`).
        #[arg(long)]
        stop_warn: bool,
        /// Claim files as they are edited (#1401): on PostToolUse claim the repo-relative path of each file an edit or
        /// write touched, for a short lease that every further edit renews, and print one line when another member
        /// holds it (never a block); on Stop and SessionEnd release those claims. Consent lives here: only a hook
        /// command that carries this flag claims anything (`klatch init --passive-claims`).
        #[arg(long)]
        passive_claims: bool,
        /// Warn before an edit (#1402): on PreToolUse print one bounded line in the context when another member holds
        /// a live claim on the path an Edit, Write, MultiEdit or NotebookEdit is about to change (names and counts
        /// only, never a reason or message text, never a block). Consent lives here: only a PreToolUse command that
        /// carries this flag does anything (`klatch init --pre-edit-warn`); without it the event is the blocking hook.
        #[arg(long)]
        pre_edit_warn: bool,
        /// Also take the passive claim before the first edit of a path (#1458), the claim PostToolUse takes after it
        /// (`klatch init --pre-edit-claims`, which needs `--passive-claims` so the claim is released at the end of the turn)
        #[arg(long, requires = "pre_edit_warn")]
        pre_edit_claims: bool,
    },
}
