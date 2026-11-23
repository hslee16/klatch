//! Running one command: each `klatch <command>` handled through any transport.

use super::*;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod agents;
mod ask_refs;
mod capped;
mod checkpoint;
mod digest_cost;
mod messaging;
mod never_read;
mod wait;
mod work;

use agents::*;
use capped::*;
use digest_cost::*;
use messaging::*;
use wait::*;
use work::*;

/// The room every session joins at session start unless told not to.
pub(crate) const DEFAULT_ROOM: &str = "lobby";

/// What a command handler knows about the caller and the moment: the transport, who it is, the client clock (used
/// only for display) and the harness session id a hook was started for.
pub(crate) struct Ctx<'a> {
    pub t: &'a dyn Transport,
    pub me: &'a str,
    pub now: i64,
    pub stdin_session: Option<&'a str>,
    /// The name was found from the coding session, not typed.
    pub discovered: bool,
    /// The whole input of a hook (a tool call for `PreToolUse`).
    pub stdin: Option<&'a serde_json::Value>,
}

/// Dispatch one command through any transport. Each family of commands has its own handler.
pub(crate) fn run(ctx: &Ctx, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Messaging(cmd) => run_messaging(ctx, cmd),
        Cmd::Wait(cmd) => run_wait(ctx, cmd),
        Cmd::Work(cmd) => run_work(ctx, cmd),
        Cmd::Agents(cmd) => run_agents(ctx, cmd),
        _ => unreachable!("handled before dispatch"),
    }
}
