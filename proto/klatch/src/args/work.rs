//! Work commands: claims, scopes and the board.

use super::*;

#[derive(Subcommand)]
pub(crate) enum WorkCmd {
    /// Claim a path prefix (advisory lease with TTL)
    #[command(display_order = 14)]
    Claim {
        room: String,
        /// ONE target: a path (quote it if it has spaces) or a name like `ticket:602`
        path: String,
        /// Extra words are refused with the form that works, never dropped (#610)
        #[arg(hide = true)]
        more: Vec<String>,
        /// Seconds the claim lasts (default 600; an `ask:<N>` claim defaults to 90 and is capped at 300, so a helper
        /// that fails frees the ask in about a minute: renew by claiming again; `merge:main`, which only an integrator or
        /// the human may take, is capped at 1800)
        #[arg(long)]
        ttl: Option<i64>,
        #[arg(long, default_value = "")]
        reason: String,
        /// Conflict with every scope, not only your own (an `ask:<N>` claim always does)
        #[arg(long)]
        shared: bool,
        /// The work item this claim is for (a factory's work order, mission or ticket id); the claim is released when
        /// a launcher reports the item done
        #[arg(long)]
        anchor: Option<String>,
    },
    /// Show the scope you work in, or set it
    #[command(display_order = 22)]
    Scope { scope: Option<String> },
    /// Show the role you hold (integrator, reviewer, ...), or none
    #[command(display_order = 34)]
    Role,
    /// Release a claim you hold
    #[command(display_order = 23)]
    Release {
        room: String,
        path: String,
        /// What you changed, for the next member to claim it (re-read before editing)
        #[arg(long)]
        note: Option<String>,
    },
    /// Ask for message SEQ of ROOM to be copied into a lower-labelled room. It waits for a verdict from the judge or
    /// the reviewer; nothing crosses until it is allowed (or redacted), and floors are checked again then
    #[command(display_order = 27)]
    Cross {
        room: String,
        seq: i64,
        #[arg(long)]
        to: String,
    },
    /// List the crossings you asked for and what became of each: waiting, approved, released, denied or blocked, with a
    /// fixed reason (the category of rule, never the text, a reviewer's words or a private term)
    #[command(display_order = 28)]
    Crossings {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// List your own recent refusals: when, the rule, the room and the operation, never the text and never another
    /// member's
    #[command(display_order = 33)]
    Refusals {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Show a room's board: open asks, current offers and members with their status
    #[command(display_order = 39)]
    Board {
        /// The room; without one, every room you have joined
        room: Option<String>,
    },
    /// Show one member's card in a room: its scope and status, the asks it posted, the asks it serves, its claims,
    /// tasks and pull requests (the member defaults to you). Derived from the board and the claims, never stored
    #[command(display_order = 40)]
    Card {
        room: String,
        /// The member to show; omit for yourself
        member: Option<String>,
    },
    /// List the asks in a room nobody has answered or closed
    #[command(display_order = 41)]
    Asks {
        /// The room; without one, every room you have joined
        room: Option<String>,
    },
    /// List the stuck cases in a room as structured events (#271): stale asks, asks nobody capable took, cycles through
    /// asks and cycles of claim waits. Read only and guarded like `read`; each line holds ids, names and numbers, never
    /// message text
    #[command(display_order = 42)]
    Stuck {
        /// The room; without one, every room you have joined
        room: Option<String>,
    },
    /// Whether a member could answer an ask: `yes`, or `no` with every reason, one line each (#671). A read only that
    /// changes nothing. Checking yourself is any member's; checking another member is the human's (an admin act)
    #[command(name = "could-answer", display_order = 44)]
    CouldAnswer {
        /// The room the ask is in
        room: String,
        /// The ask's number (the `#N` that `klatch asks` shows)
        ask: i64,
        /// The member to check; omit for yourself
        member: Option<String>,
    },
    /// List the open durable tasks in a room with their full latest checkpoint (working, checkpoint, handoff)
    #[command(display_order = 43)]
    Tasks { room: String },
    /// List active claims in a room
    #[command(display_order = 44)]
    Claims {
        /// The room; without one, every room you have joined
        room: Option<String>,
    },
}
