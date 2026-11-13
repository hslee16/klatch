//! klatch: local prototype of the agent rooms gateway (DESIGN.md M1 thin slice).
//!
//! Identity is the KLATCH_NAME env var, which is FORGEABLE (prototype only; see DESIGN.md 8.4).
//! All state goes through the `Store` trait; this file only parses arguments and prints.
mod admin;
mod commands;
mod identity;
mod init;
mod scope;
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use klatch_core::{
    Principal, convention, flat, hook_output_json, render_board, render_hook, render_who,
};
use klatch_gateway::{AllowAll, Direct, Gateway, Identity, LocalIdentity, PeerEvidence, Transport};
use klatch_harness::{Found, FsRegistry, Miss, PsTable, find_session};
use klatch_store::{SqliteStore, Store};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use commands::*;
use identity::*;
use scope::*;

#[derive(Parser)]
#[command(name = "klatch", about = "Local agent rooms prototype", version = VERSION)]
struct Cli {
    #[command(flatten)]
    who: WhoFlags,
    #[command(subcommand)]
    cmd: Cmd,
}

/// The flags that say who is calling.
#[derive(clap::Args)]
struct WhoFlags {
    /// Member name (default: $KLATCH_NAME)
    #[arg(long, global = true)]
    as_member: Option<String>,
    /// Find this process's coding-agent session from its process tree and the harness registry when no name is
    /// given (default: off, or set KLATCH_DISCOVER=1). Reads only the registry's .json files, never a key file.
    #[arg(long, global = true)]
    discover: bool,
    /// The scope this member works in (default: $KLATCH_SCOPE, else a coding session's repository). Claims conflict
    /// only inside one scope, so two repositories or two bots can each claim `src/lib.rs`.
    #[arg(long = "member-scope", global = true)]
    member_scope: Option<String>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Join a room (idempotent). Unread starts at the beginning of history.
    Join { room: String },
    /// Post a message to a room
    Post { room: String, body: Vec<String> },
    /// Read unread messages in a room and advance the cursor
    Read {
        room: String,
        /// Do not advance the cursor
        #[arg(long)]
        peek: bool,
    },
    /// Wait until another member posts in a room (or the timeout), then print and mark read what arrived
    Wait {
        /// Rooms to wait on; wakes on the first with a matching message
        #[arg(required = true)]
        rooms: Vec<String>,
        /// Seconds to wait (default 60, at most 300); 0 checks once
        #[arg(long, default_value_t = 60)]
        timeout: u64,
        /// Only wake for messages whose body starts with this text; others stay unread
        #[arg(long)]
        only: Option<String>,
        /// Only wake for messages from this member; others stay unread
        #[arg(long)]
        from: Option<String>,
        /// Return the messages without marking them read
        #[arg(long)]
        peek: bool,
    },
    /// List members of a room with last activity
    Who {
        room: String,
        /// List every member, including those not seen for over 10 minutes
        #[arg(long)]
        all: bool,
    },
    /// Claim a path prefix (advisory lease with TTL)
    Claim {
        room: String,
        path: String,
        /// Seconds the claim lasts (default 600; an `ask:<N>` claim defaults to 90 and is capped at 300, so a helper
        /// that fails frees the ask in about a minute: renew by claiming again)
        #[arg(long)]
        ttl: Option<i64>,
        #[arg(long, default_value = "")]
        reason: String,
        /// Conflict with every scope, not only your own (an `ask:<N>` claim always does)
        #[arg(long)]
        shared: bool,
    },
    /// Controls for the person running klatch: pause, close, remove a member, audit (needs a real terminal)
    Admin {
        #[command(subcommand)]
        cmd: admin::AdminCmd,
    },
    /// Show the scope you work in, or set it
    Scope { scope: Option<String> },
    /// Release a claim you hold
    Release {
        room: String,
        path: String,
        /// What you changed, for the next member to claim it (re-read before editing)
        #[arg(long)]
        note: Option<String>,
    },
    /// Set your status in a room (for example "done"), shown by who and the board
    Status { room: String, status: Vec<String> },
    /// Show when each hook event last ran for you (or for another member with --of), without any content
    Hooks {
        /// A member name to look at instead of yourself
        #[arg(long)]
        of: Option<String>,
    },
    /// Install klatch's hooks into Claude Code settings, after showing a diff and getting a yes per scope
    Init {
        /// Where to write: project (.claude/settings.local.json here), user (~/.claude/settings.json) or both
        #[arg(long, value_enum)]
        scope: Option<init::Scope>,
        /// Show this room's board at session start, without joining it (the consent to read a room first)
        #[arg(long)]
        board: Option<String>,
        /// Ask nothing: the flags are the consent. Needs --scope; never turns the board on by itself
        #[arg(long)]
        yes: bool,
        /// Do not join new sessions to the default lobby at session start (the default is to join)
        #[arg(long)]
        no_lobby: bool,
        /// Do not install the SessionEnd hook that marks a finished session done (the default is to install it)
        #[arg(long)]
        no_session_end: bool,
    },
    /// Show a room's board: open asks, current offers and members with their status
    Board { room: String },
    /// List the asks in a room nobody has answered or closed
    Asks { room: String },
    /// List active claims in a room
    Claims { room: String },
    /// Hook output for UserPromptSubmit/SessionStart: unread messages and claims across joined rooms
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
    },
    /// Write the event stream of a room to standard output as newline-delimited JSON (PROTOCOL.md)
    Stream {
        room: String,
        /// Start after this per-room sequence number (0 for the beginning)
        #[arg(long, default_value_t = 0)]
        from_seq: u64,
        /// Keep running and print new messages as they arrive, instead of exiting once caught up
        #[arg(long)]
        follow: bool,
        /// How often to poll for new messages, in milliseconds
        #[arg(long, default_value_t = 100, hide = true)]
        poll_ms: u64,
    },
    /// Speak the wire protocol (PROTOCOL.md) on standard input and output, as the member from the environment
    Serve {
        /// Use standard input and output (the only transport so far)
        #[arg(long)]
        stdio: bool,
    },
}

/// The time klatch acts on. A test build (`--features test-clock`) takes it from `KLATCH_NOW` (unix seconds) when
/// that is set, so time-dependent behaviour can be tested end to end; a real build has no such seam and ignores
/// the variable (DESIGN section 26.5).
fn now() -> i64 {
    #[cfg(feature = "test-clock")]
    if let Some(t) = std::env::var("KLATCH_NOW")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        return t;
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// The clock the gateway uses: the same time as `now()`.
struct CliClock;

impl klatch_gateway::Clock for CliClock {
    fn now(&self) -> i64 {
        now()
    }
}

/// The version text; a test build says so, which is how the bench knows it has the right binary.
const VERSION: &str = if cfg!(feature = "test-clock") {
    concat!(env!("CARGO_PKG_VERSION"), " (test-clock)")
} else {
    env!("CARGO_PKG_VERSION")
};

fn db_path() -> Result<PathBuf> {
    let base = match std::env::var("KLATCH_HOME") {
        Ok(p) => PathBuf::from(p),
        Err(_) => PathBuf::from(std::env::var("HOME").context("HOME not set")?).join(".klatch"),
    };
    std::fs::create_dir_all(&base)?;
    Ok(base.join("rooms.db"))
}

/// A hook that cannot write the store (it is read-only) must not fail every turn of a session: it says so in one
/// line and exits 0, changing nothing. Any other failure stays loud (ticket #291, DESIGN section 26.4).
const READ_ONLY_NOTICE: &str = "klatch: the store is read-only, so the lobby is unavailable";

fn is_read_only(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        let m = c.to_string().to_lowercase();
        m.contains("readonly database") || m.contains("read-only")
    })
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let hook_event = match &cli.cmd {
        Cmd::Hook { event, .. } => Some(event.clone().unwrap_or_default()),
        _ => None,
    };
    match run_cli(cli) {
        Err(e) if hook_event.is_some() && is_read_only(&e) => {
            // the session is over at SessionEnd: nothing to tell it
            if hook_event.as_deref() != Some("SessionEnd") {
                print!(
                    "{}",
                    klatch_core::hook_output_json(
                        hook_event.as_deref().unwrap_or(""),
                        READ_ONLY_NOTICE,
                        None
                    )
                );
            }
            Ok(())
        }
        other => other,
    }
}

fn run_cli(cli: Cli) -> Result<()> {
    match cli.cmd {
        Cmd::Init {
            scope,
            board,
            yes,
            no_lobby,
            no_session_end,
        } => run_init(&init::Options {
            scope,
            board,
            yes,
            no_lobby,
            no_session_end,
        }),
        Cmd::Admin { cmd } => admin::run(cmd),
        cmd => run_as_member(&cli.who, cmd),
    }
}

fn run_init(options: &init::Options) -> Result<()> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    init::run(
        options,
        &mut init::Io {
            input: &mut std::io::stdin().lock(),
            out: &mut std::io::stdout().lock(),
        },
        &init::Places {
            project_dir: &std::env::current_dir()?,
            home: home.as_deref(),
            bin: &std::env::current_exe()?,
        },
    )
}

/// Every command that acts as a member: find out which, settle its scope, then serve, stream or run the command.
fn run_as_member(who: &WhoFlags, cmd: Cmd) -> Result<()> {
    let store = SqliteStore::open(&db_path()?)?;
    let Some(caller) = identify(who, &cmd, &store)? else {
        return Ok(());
    };
    // The scope a member works in: the one it declared (flag or KLATCH_SCOPE), else the one it already has, else
    // for a coding session the repository it started in (DESIGN section 27).
    let declared_scope = who
        .member_scope
        .clone()
        .or_else(|| std::env::var("KLATCH_SCOPE").ok());
    ensure_member_scope(
        &store,
        &caller.principal.member,
        (declared_scope.as_deref(), caller.session_dir.as_deref()),
        now(),
    )?;
    let gateway = Rc::new(Gateway::new(store, CliClock, AllowAll));
    match cmd {
        Cmd::Serve { stdio } => serve_stdio(&gateway, &caller.principal, stdio),
        Cmd::Stream {
            room,
            from_seq,
            follow,
            poll_ms,
        } => {
            let config = klatch_session::SessionConfig {
                poll: std::time::Duration::from_millis(poll_ms),
                ..Default::default()
            };
            // stdout carries protocol frames only
            Ok(klatch_session::stream(
                &gateway,
                &caller.principal,
                &room,
                from_seq,
                follow,
                std::io::stdout().lock(),
                &config,
            )?)
        }
        cmd => {
            let me = caller.principal.member.clone();
            let transport = Direct::new(gateway, caller.principal);
            let ctx = Ctx {
                t: &transport,
                me: &me,
                now: now(),
                stdin_session: caller.stdin_session.as_deref(),
            };
            run(&ctx, cmd)
        }
    }
}

fn serve_stdio(gateway: &Gateway<SqliteStore>, principal: &Principal, stdio: bool) -> Result<()> {
    if !stdio {
        bail!("serve needs --stdio, the only transport so far");
    }
    // stdout carries protocol frames only; nothing else may print there
    Ok(klatch_session::serve(
        gateway,
        principal,
        std::io::BufReader::new(std::io::stdin()),
        std::io::stdout().lock(),
    )?)
}
