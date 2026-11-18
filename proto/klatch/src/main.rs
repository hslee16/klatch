//! klatch: local prototype of the agent rooms gateway (DESIGN.md M1 thin slice).
//!
//! Identity is the KLATCH_NAME env var, which is FORGEABLE (prototype only; see DESIGN.md 8.4).
//! All state goes through the `Store` trait; this file only parses arguments and prints.
mod admin;
mod anchor;
mod codex;
mod commands;
mod consent;
mod custody;
mod doctor;
mod identity;
mod init;
mod init_codex;
mod judge;
mod roles;
mod run;
mod scope;
mod worker;
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use klatch::domain::{
    Asker, Claim, Held, Principal, ToolCall, ToolVerdict, convention, flat, hook_output_json,
    judge_tool, patch_targets, render_board, render_hook, render_who,
};
use klatch::gateway::{
    AllowAll, ClaimSpec, Direct, ForbidFloors, Gateway, Identity, LocalIdentity, PeerEvidence,
    Transport,
};
use klatch::harness::{Found, FsRegistry, Miss, PsTable, find_parent, find_session};
use klatch::store::SqliteStore;
use klatch::store::areas::*;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use commands::*;
use identity::*;
use scope::*;

#[derive(Parser)]
#[command(name = "klatch", about = "Local agent rooms prototype", version = version_text())]
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
    /// This is a Codex session (it has no registry record): when no name is given, the member comes from the hook's
    /// session id and working directory on standard input, or from CODEX_THREAD_ID for a command. A resumed session
    /// keeps its member.
    #[arg(long, global = true)]
    codex: bool,
    /// The scope this member works in (default: $KLATCH_SCOPE, else a coding session's repository). Claims conflict
    /// only inside one scope, so two repositories or two bots can each claim `src/lib.rs`.
    #[arg(long = "member-scope", global = true)]
    member_scope: Option<String>,
    /// What sort of member this is: bot, service or human (default: $KLATCH_KIND; `off` withdraws it). A coding
    /// session is a session by itself. Shown by `who` and the board, and it changes who may do what for no one.
    #[arg(long = "member-kind", global = true)]
    member_kind: Option<String>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Join a room (idempotent). Unread starts at the beginning of history.
    Join { room: String },
    /// Send a direct message to one member, in a room only the two of you can use
    Dm {
        member: String,
        body: Vec<String>,
        /// A label lower than the room's ceiling (the lower of the two clearances)
        #[arg(long)]
        label: Option<String>,
    },
    /// Post a message to a room
    Post {
        room: String,
        body: Vec<String>,
        /// A label lower than the room's own, for a message that is less sensitive than the room
        #[arg(long)]
        label: Option<String>,
        /// Make this a handoff: a short kind (letters, digits, dot, dash, underscore) that klatch stores and
        /// delivers and never interprets
        #[arg(long)]
        kind: Option<String>,
        /// An opaque payload for the next member (at most 16 KiB; needs --kind); held to the same rules as the body
        #[arg(long, requires = "kind")]
        payload: Option<String>,
    },
    /// Read unread messages in a room and advance the cursor
    Read {
        room: String,
        /// Do not advance the cursor
        #[arg(long)]
        peek: bool,
    },
    /// Look back in a room: the newest 20 messages, or `--last N`, or the next 20 after `--since SEQ`. Prints
    /// everyone's messages as `read` does and moves no read cursor.
    History {
        /// The room; without one, every room you have joined
        room: Option<String>,
        /// The newest N messages (at most 200); with `--since`, the newest N after it
        #[arg(long)]
        last: Option<usize>,
        /// Only messages after this seq; alone, the next 20, so a long room can be paged
        #[arg(long)]
        since: Option<i64>,
    },
    /// Wait until another member posts in a room (or the timeout), then print and mark read what arrived
    Wait {
        /// Rooms to wait on; wakes on the first with a matching message. A room may carry its own filter,
        /// `ROOM:only=PREFIX` or `ROOM:from=MEMBER` (name the room again for another), which replaces --only and
        /// --from for that room, so a room of two can wake on anything while the lobby is filtered
        #[arg(required = true)]
        rooms: Vec<String>,
        /// Seconds to wait (default 60, at most 300); 0 checks once
        #[arg(long, default_value_t = 60)]
        timeout: u64,
        /// Only wake for messages whose body starts with this text; others stay unread. Repeat it to wake on any
        /// of several prefixes
        #[arg(long)]
        only: Vec<String>,
        /// Only wake for messages from this member; others stay unread
        #[arg(long)]
        from: Option<String>,
        /// Return the messages without marking them read
        #[arg(long)]
        peek: bool,
    },
    /// Invite a member to a room: post a content-free pointer (`klatch join <room>`) in a room it is already in.
    /// It joins nobody; the invited session decides. It reaches a session through the hooks or the watcher it has.
    Invite {
        member: String,
        room: String,
        /// The room both of you are in, where the pointer is posted
        #[arg(long, default_value = "lobby")]
        via: String,
    },
    /// Watch for unread messages and, when one arrives, exit 2 with a one-line pointer (no content): the command
    /// of an `asyncRewake` hook. Quiet at its timeout. Moves no read cursor; refreshes presence.
    Wake {
        /// Seconds to watch (default 3500, at most 3600)
        #[arg(long, default_value_t = 3500)]
        timeout: u64,
    },
    /// List members of a room with last activity
    Who {
        /// The room; without one, every room you have joined
        room: Option<String>,
        /// List every member, including those not seen for over 10 minutes
        #[arg(long)]
        all: bool,
    },
    /// Show what you can learn about each recipient of your message SEQ in ROOM: parked, seen active after the post,
    /// or not seen since. Liveness only (what `who` already shows); not proof anyone was shown or read it
    Receipt { room: String, seq: i64 },
    /// Claim a path prefix (advisory lease with TTL)
    Claim {
        room: String,
        /// ONE target: a path (quote it if it has spaces) or a name like `ticket:602`
        path: String,
        /// Extra words are refused with the form that works, never dropped (#610)
        #[arg(hide = true)]
        more: Vec<String>,
        /// Seconds the claim lasts (default 600; an `ask:<N>` claim defaults to 90 and is capped at 300, so a helper
        /// that fails frees the ask in about a minute: renew by claiming again)
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
    /// Issue, revoke and list the credentials of workers (for a launcher; see `klatch worker --help`)
    Worker {
        #[command(subcommand)]
        cmd: worker::WorkerCmd,
    },
    /// Report a factory's work item done, or poll for finished ones, and release the claims anchored to them (for a
    /// launcher; see `klatch anchor --help`)
    Anchor {
        #[command(subcommand)]
        cmd: anchor::AnchorCmd,
    },
    /// Open and end a factory run, whose rooms are `run.<id>.<name>` (for a launcher; see `klatch run --help`)
    Run {
        #[command(subcommand)]
        cmd: run::RunCmd,
    },
    /// Controls for the person running klatch: pause, close, remove a member, audit (needs a real terminal)
    Admin {
        #[command(subcommand)]
        cmd: admin::AdminCmd,
    },
    /// Run the review queue through the judge (for a launcher; see `klatch judge --help`)
    Judge {
        #[command(subcommand)]
        cmd: judge::JudgeCmd,
    },
    /// Call the acts your role allows, as yourself (admin agent; not `klatch admin`, which stays the person's)
    Act {
        #[command(subcommand)]
        cmd: roles::ActCmd,
    },
    /// Read the review queue and record verdicts (reviewer only)
    Review {
        #[command(subcommand)]
        cmd: roles::ReviewCmd,
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
    /// Mute a room: your hook then shows only messages that mention you as @your-name (`read` still shows all)
    Mute { room: String },
    /// Unmute a room
    Unmute { room: String },
    /// Show or set what your hook and wake show without being asked: `all` (the default) or `scope` (only your own
    /// scope, messages addressed to you as @your-name, and broadcasts as @all). `read` and `history` still show all
    View {
        /// `all` or `scope`; omit to show the current view
        view: Option<String>,
    },
    /// Ask for message SEQ of ROOM to be copied into a lower-labelled room. It waits for a verdict from the judge or
    /// the reviewer; nothing crosses until it is allowed (or redacted), and floors are checked again then
    Cross {
        room: String,
        seq: i64,
        #[arg(long)]
        to: String,
    },
    /// Set your status in a room (for example "done"), shown by who and the board
    Status { room: String, status: Vec<String> },
    /// Run the reference router as your own member (#189): read the lobby's open asks, claims and presence and invite
    /// the members that hold a claim on an asker's topic into a topic room. Rules only, no model; it needs the
    /// create-room and invite capability like any member, and it decides nothing from message text.
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
    /// Show when each hook event last ran for you (or for another member with --of), without any content
    Hooks {
        /// A member name to look at instead of yourself
        #[arg(long)]
        of: Option<String>,
    },
    /// List your own recent refusals: when, the rule, the room and the operation, never the text and never another
    /// member's
    Refusals {
        #[arg(long, default_value_t = 20)]
        limit: usize,
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
        /// Install the PreToolUse hook that blocks an edit of a path another member claimed (off by default)
        #[arg(long, conflicts_with = "no_enforce")]
        enforce: bool,
        /// Remove klatch's PreToolUse hook
        #[arg(long)]
        no_enforce: bool,
        /// Install the asyncRewake hook that wakes an idle session for unread messages (off by default)
        #[arg(long, conflicts_with = "no_wake")]
        wake: bool,
        /// Remove klatch's wake hook
        #[arg(long)]
        no_wake: bool,
        /// Remove every hook klatch installed (and nothing else) from the settings of --scope; keeps your rooms
        /// and keys
        #[arg(
            long,
            conflicts_with_all = ["board", "no_lobby", "no_session_end", "enforce", "no_enforce", "wake", "no_wake"]
        )]
        uninstall: bool,
    },
    /// Show the consents `klatch init` granted (board, join, session-end, enforce, wake), or switch one on or off
    /// later; edits only klatch's own hook entries, after a diff and a yes
    Consent {
        /// The consent to change: board, join, session-end, enforce or wake (none: show them all)
        name: Option<String>,
        /// on or off (for board: off or a room name)
        value: Option<String>,
        /// Which settings to read or change: project, user or both (a change needs it)
        #[arg(long, value_enum)]
        scope: Option<init::Scope>,
        /// Ask nothing: the flags are the consent
        #[arg(long)]
        yes: bool,
    },
    /// Print the klatch convention for any agent, to paste into a subagent's brief (needs no store and no name)
    Convention {
        /// The member name the child will use (letters, digits, - _ .); without one the text asks the child to pick
        #[arg(long)]
        name: Option<String>,
        /// The room the child works in (default lobby), named in every command of the brief
        #[arg(long)]
        room: Option<String>,
    },
    /// Check this installation: the binary, the store and its key, the hooks, the session (read only; creates nothing)
    Doctor,
    /// List the live coding-agent sessions on this machine from the harness registry (read only; never a key file)
    Sessions {
        /// Also list the records that are not live, with the reason, and how many files could not be read
        #[arg(long)]
        all: bool,
    },
    /// Show a room's board: open asks, current offers and members with their status
    Board {
        /// The room; without one, every room you have joined
        room: Option<String>,
    },
    /// Show one member's card in a room: its scope and status, the asks it posted, the asks it serves, its claims,
    /// tasks and pull requests (the member defaults to you). Derived from the board and the claims, never stored
    Card {
        room: String,
        /// The member to show; omit for yourself
        member: Option<String>,
    },
    /// List the asks in a room nobody has answered or closed
    Asks {
        /// The room; without one, every room you have joined
        room: Option<String>,
    },
    /// List the open durable tasks in a room with their full latest checkpoint (working, checkpoint, handoff)
    Tasks { room: String },
    /// List active claims in a room
    Claims {
        /// The room; without one, every room you have joined
        room: Option<String>,
    },
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
    /// Run an MCP (Model Context Protocol) server on standard input and output as the member from the environment, so
    /// an agent runtime that can attach a tool server can post, read, wait, claim and release without a harness
    Mcp {
        /// Use standard input and output (the only transport so far)
        #[arg(long)]
        stdio: bool,
    },
    /// Offer the read-only librarian to an editor as an ACP (Agent Client Protocol) agent on standard input and output,
    /// as the member from the environment; it can only look (read without marking read, history, who, claims)
    Acp {
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

impl klatch::gateway::Clock for CliClock {
    fn now(&self) -> i64 {
        now()
    }
}

/// The version text: the package version, then the wire protocol and the store schema this binary speaks (a store
/// written by a newer schema is refused, so this says whether the binary can open yours). A test build says so too,
/// which is how the bench knows it has the right binary.
pub(crate) fn version_text() -> &'static str {
    static TEXT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TEXT.get_or_init(|| {
        let test_build = if cfg!(feature = "test-clock") {
            ", test-clock"
        } else {
            ""
        };
        format!(
            "{} (protocol {}, store schema {}{test_build})",
            env!("CARGO_PKG_VERSION"),
            klatch::protocol::PROTOCOL_VERSION,
            klatch::store::SCHEMA_VERSION
        )
    })
}

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

/// The one line a hook prints when it could not read the rooms (R19, #34): in the agent's context, so a session
/// does not take silence for "nothing new". The reason is the root cause only, on one line and bounded.
fn failure_line(e: &anyhow::Error, event: &str) -> String {
    if is_read_only(e) {
        return READ_ONLY_NOTICE.to_string();
    }
    let why: String = klatch::domain::flat(&e.root_cause().to_string())
        .chars()
        .take(120)
        .collect();
    if event == "PreToolUse" {
        return format!(
            "[klatch] could not check claims ({why}); this call was not checked against other members' claims"
        );
    }
    format!(
        "[klatch] could not read the rooms ({why}); messages may be waiting. Try `klatch read <room>` or tell your user"
    )
}

/// Keep content and keys out of crash output (R37): a panic prints where it happened and nothing it carried (a
/// payload can hold text a caller sent), and a crash leaves no core file (it would hold keys and messages in memory).
fn harden() {
    std::panic::set_hook(Box::new(|info| {
        let at = info.location().map_or_else(
            || "unknown place".to_string(),
            |l| format!("{}:{}", l.file(), l.line()),
        );
        eprintln!("klatch: internal error at {at}; this is a bug (nothing you sent is shown here)");
    }));
    disable_core_dumps();
}

fn disable_core_dumps() {
    let zero = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // best effort: failing to lower a limit is not a reason to stop
    unsafe {
        libc::setrlimit(libc::RLIMIT_CORE, &zero);
    }
}

fn main() -> Result<()> {
    harden();
    #[cfg(feature = "test-clock")]
    if let Ok(text) = std::env::var("KLATCH_TEST_PANIC") {
        panic!("{text}");
    }
    let cli = Cli::parse();
    let hook_event = match &cli.cmd {
        Cmd::Hook { event, .. } => Some(event.clone()),
        _ => None,
    };
    let names_installed = !matches!(
        cli.cmd,
        Cmd::Init { .. }
            | Cmd::Consent { .. }
            | Cmd::Doctor
            | Cmd::Convention { .. }
            | Cmd::Sessions { .. }
    );
    match run_cli(cli) {
        // A hook never fails a session: it says what went wrong where the model reads it, and exits 0. The session
        // is over at SessionEnd, so there is nothing to tell it.
        Err(e) if hook_event.is_some() => {
            let event = hook_event.flatten();
            let line = failure_line(&e, event.as_deref().unwrap_or(""));
            match event.as_deref() {
                Some("SessionEnd") => {}
                Some(event) => print!(
                    "{}",
                    klatch::domain::hook_output_json(event, &line, Some(&line))
                ),
                None => println!("{line}"),
            }
            Ok(())
        }
        Err(e) => {
            if let Some(line) = other_binary_line().filter(|_| names_installed) {
                eprintln!("{line}");
            }
            Err(e)
        }
        ok => ok,
    }
}

/// When the failing binary is not the installed one, the line that names the installed one (#513).
fn other_binary_line() -> Option<String> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    init::other_binary_note(
        &std::env::current_exe().ok()?,
        &std::env::current_dir().ok()?,
        home.as_deref(),
    )
}

fn run_cli(cli: Cli) -> Result<()> {
    match cli.cmd {
        Cmd::Init {
            scope,
            board,
            yes,
            no_lobby,
            no_session_end,
            enforce,
            no_enforce,
            wake,
            no_wake,
            uninstall,
        } => run_init(&init::Options {
            scope,
            board,
            yes,
            no_lobby,
            no_session_end,
            enforce: init::Switch::from_flags(enforce, no_enforce),
            wake: init::Switch::from_flags(wake, no_wake),
            uninstall,
            codex: cli.who.codex,
        }),
        Cmd::Consent {
            name,
            value,
            scope,
            yes,
        } => run_consent(&consent::Options {
            scope,
            name,
            value,
            yes,
        }),
        Cmd::Sessions { all } => run_sessions(all),
        Cmd::Doctor => run_doctor(),
        Cmd::Convention { name, room } => run_convention(name.as_deref(), room.as_deref()),
        Cmd::Admin { cmd } => admin::run(cmd),
        Cmd::Worker { cmd } => worker::run(cmd),
        Cmd::Anchor { cmd } => anchor::run(cmd),
        Cmd::Run { cmd } => run::run(cmd),
        Cmd::Judge { cmd } => judge::run(cmd),
        Cmd::Wake { .. } if !wake_is_on() => Ok(()),
        cmd => run_as_member(&cli.who, cmd),
    }
}

/// Whether the wake watcher may run: a harness probe that says the coding agent is too old for `asyncRewake` turns it
/// off (and says so on standard error, which an async hook never shows); one that cannot tell leaves it on (#482).
fn wake_is_on() -> bool {
    let support = klatch::harness::wake_support_here();
    if !support.wake_enabled() {
        eprintln!("klatch: {}", support.say());
    }
    support.wake_enabled()
}

/// `klatch convention`: the brief for a child agent, with this binary's own path.
fn run_convention(name: Option<&str>, room: Option<&str>) -> Result<()> {
    if let Some(n) = name.filter(|n| !klatch::domain::valid_name(n)) {
        bail!("the name '{n}' is not a member name (1 to 40 letters, digits, - _ .)");
    }
    if let Some(r) = room.filter(|r| !klatch::domain::valid_name(r)) {
        bail!("the room '{r}' is not a room name (1 to 40 letters, digits, - _ .)");
    }
    let bin = std::env::current_exe()?;
    print!(
        "{}",
        klatch::domain::subagent_brief(&bin.to_string_lossy(), name, room)
    );
    Ok(())
}

/// `klatch doctor`: exit 1 when a probe fails.
fn run_doctor() -> Result<()> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let healthy = doctor::run(
        &std::env::current_dir()?,
        home.as_deref(),
        &std::env::current_exe()?,
        &mut std::io::stdout().lock(),
    )?;
    if !healthy {
        std::process::exit(1);
    }
    Ok(())
}

/// `klatch sessions`: one tab separated line per live session (pid, name, directory, session id); with `--all` also the
/// records that are not live and why.
fn run_sessions(all: bool) -> Result<()> {
    use klatch::harness::Liveness;
    let Some((registry, domain)) = identity::registry_and_domain() else {
        bail!("HOME is not set, so the harness registry cannot be found");
    };
    let found = klatch::harness::list(&registry, &PsTable, domain);
    for r in &found.live {
        println!(
            "{}\t{}\t{}\t{}",
            r.pid,
            r.name.as_deref().unwrap_or("-"),
            r.cwd,
            r.session_id
        );
    }
    if found.live.is_empty() {
        eprintln!("no live sessions");
    }
    if all {
        for (r, why) in &found.ignored {
            let why = match why {
                Liveness::Dead => "not running",
                Liveness::Reused => "pid reused by another process",
                Liveness::NotHarness => "running, but not the harness",
                Liveness::OtherDomain => "recorded on another machine",
                Liveness::Live => "live",
                Liveness::Unreadable => "could not be checked (the process table cannot be read)",
            };
            println!(
                "ignored\t{}\t{why}\t{}\t{}",
                r.pid,
                r.name.as_deref().unwrap_or("-"),
                r.cwd
            );
        }
        println!("unreadable\t{}", found.malformed);
    } else if !found.ignored.is_empty() || found.malformed > 0 {
        eprintln!(
            "{} other record(s) ignored (not running, a reused pid, not the harness or another machine) and {} unreadable; --all lists them",
            found.ignored.len(),
            found.malformed
        );
    }
    Ok(())
}

/// `klatch consent`: show or switch the consents init granted.
fn run_consent(options: &consent::Options) -> Result<()> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    consent::run(
        options,
        &mut init::Io {
            input: &mut std::io::stdin().lock(),
            out: &mut std::io::stdout().lock(),
        },
        &init::Places {
            project_dir: &std::env::current_dir()?,
            home: home.as_deref(),
            bin: &std::env::current_exe()?,
            codex_home: None,
        },
    )
}

fn run_init(options: &init::Options) -> Result<()> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let codex_home = codex::codex_home(std::env::var_os("CODEX_HOME"), home.as_deref());
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
            codex_home: codex_home.as_deref(),
        },
    )?;
    // A klatch hook that Codex has not trusted is skipped without a word (#544), so say so while the human is here.
    if !options.uninstall
        && let Some(dir) = codex_home
    {
        codex::init_notice(
            &dir,
            &std::env::current_dir()?,
            &mut std::io::stdout().lock(),
        )?;
    }
    Ok(())
}

/// Every command that acts as a member: find out which, settle its scope, then serve, stream or run the command.
fn run_as_member(who: &WhoFlags, cmd: Cmd) -> Result<()> {
    let store = custody::open_store(&db_path()?)?;
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
    let parent = pending_parent(&store, &caller);
    let mut gateway = Gateway::new(store, CliClock, ForbidFloors::new(AllowAll)).for_members();
    // claims follow their holder's liveness (DESIGN section 29, #304); no home means no registry to probe
    if let Some(home) = std::env::var_os("HOME") {
        gateway = gateway.with_leases(klatch::gateway::SysLeaseSource::under_home(
            std::path::Path::new(&home),
        ));
    }
    let gateway = Rc::new(gateway);
    if let Some(kind) = declared_kind(who.member_kind.as_deref()) {
        declare_member_kind(&gateway, &caller.principal, &kind)?;
    }
    // the keys of the sealed labels this member's clearance reaches, and no others (#160)
    let labels = gateway.sealed_labels_within(&caller.principal.member)?;
    if !labels.is_empty() {
        gateway.use_label_keys(custody::label_ring(&db_path()?, &labels)?);
    }
    if let Some(parent) = parent {
        // a failure to note it must never stop the command
        let _ = gateway.note_parent(&caller.principal, parent.as_deref());
    }
    match cmd {
        Cmd::Serve { stdio } => serve_stdio(&gateway, &caller.principal, stdio),
        Cmd::Mcp { stdio } => mcp_stdio(&gateway, &caller.principal, stdio),
        Cmd::Acp { stdio } => acp_stdio(&gateway, &caller.principal, stdio),
        Cmd::Stream {
            room,
            from_seq,
            follow,
            poll_ms,
        } => stream_room(
            (&gateway, &caller.principal),
            &klatch::session::StreamRequest {
                room: &room,
                from_seq,
                follow,
            },
            poll_ms,
        ),
        cmd => {
            let me = caller.principal.member.clone();
            let mut transport = Direct::new(gateway, caller.principal);
            if caller.claude_code {
                transport = transport.with_harness(Box::new(klatch::harness::ClaudeCode::probing(
                    klatch::harness::wake_support_here,
                )));
            }
            let ctx = Ctx {
                t: &transport,
                me: &me,
                now: now(),
                stdin_session: caller.stdin_session.as_deref(),
                discovered: caller.session.is_some(),
                stdin: caller.stdin.as_ref(),
            };
            run(&ctx, cmd)
        }
    }
}

/// `klatch stream`: one room's events on standard output, which carries protocol frames only.
fn stream_room(
    (gateway, principal): (&Gateway<SqliteStore>, &Principal),
    request: &klatch::session::StreamRequest,
    poll_ms: u64,
) -> Result<()> {
    let config = klatch::session::SessionConfig {
        poll: std::time::Duration::from_millis(poll_ms),
        ..Default::default()
    };
    klatch::session::Session::new(gateway, principal, &config)
        .stream(request, std::io::stdout().lock())
        .map_err(Into::into)
}

fn serve_stdio(gateway: &Gateway<SqliteStore>, principal: &Principal, stdio: bool) -> Result<()> {
    if !stdio {
        bail!("serve needs --stdio, the only transport so far");
    }
    // stdout carries protocol frames only; nothing else may print there
    // stdin cannot be stopped from outside; the process exits when the session does
    let config = klatch::session::SessionConfig::default();
    Ok(
        klatch::session::Session::new(gateway, principal, &config).serve_guarded(
            std::io::BufReader::new(std::io::stdin()),
            std::io::stdout(),
            None,
        )?,
    )
}

/// `klatch mcp --stdio`: the MCP front end over the same session `serve` uses; standard output carries MCP messages only.
fn mcp_stdio(gateway: &Gateway<SqliteStore>, principal: &Principal, stdio: bool) -> Result<()> {
    if !stdio {
        bail!("mcp needs --stdio, the only transport so far");
    }
    let config = klatch::session::SessionConfig::default();
    let session = klatch::session::Session::new(gateway, principal, &config);
    let bin = std::env::current_exe()?;
    let instructions = klatch::domain::subagent_brief(
        &bin.to_string_lossy(),
        Some(&principal.member),
        Some(commands::DEFAULT_ROOM),
    );
    klatch::session::mcp::McpServer::new(&session, instructions)
        .serve(std::io::stdin().lock(), std::io::stdout())?;
    Ok(())
}

/// `klatch acp --stdio`: the librarian as an ACP agent over the same session `serve` uses (#115).
fn acp_stdio(gateway: &Gateway<SqliteStore>, principal: &Principal, stdio: bool) -> Result<()> {
    if !stdio {
        bail!("acp needs --stdio, the only transport so far");
    }
    let config = klatch::session::SessionConfig::default();
    let session = klatch::session::Session::new(gateway, principal, &config);
    klatch::session::acp::AcpServer::new(&session)
        .serve(std::io::stdin().lock(), std::io::stdout())?;
    Ok(())
}

#[cfg(test)]
mod hardening_tests {
    #[test]
    fn core_dumps_are_disabled() {
        super::disable_core_dumps();
        let mut now = libc::rlimit {
            rlim_cur: 1,
            rlim_max: 1,
        };
        unsafe {
            libc::getrlimit(libc::RLIMIT_CORE, &mut now);
        }
        assert_eq!((now.rlim_cur, now.rlim_max), (0, 0));
    }
}
