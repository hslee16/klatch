//! klatch: local prototype of the agent rooms gateway (DESIGN.md M1 thin slice).
//!
//! Identity is the KLATCH_NAME env var, which is FORGEABLE (prototype only; see DESIGN.md 8.4).
//! All state goes through the `Store` trait; this file only parses arguments and prints.
mod admin;
mod anchor;
mod args;
mod codex;
mod commands;
mod consent;
mod doctor;
mod init;
mod init_codex;
mod judge;
mod roles;
mod run;
/// The scratch-directory guard of the library's store tests, compiled into the binary's tests too (#812).
#[cfg(test)]
#[allow(dead_code)]
#[path = "store/test_dir.rs"]
mod test_dir;
mod worker;
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use klatch::domain::{
    Asker, Claim, Held, Principal, ToolCall, ToolVerdict, convention, flat, hook_output_json,
    judge_tool, patch_targets, render_board, render_who,
};
use klatch::gateway::{
    AllowAll, ClaimSpec, Direct, ForbidFloors, Gateway, RequireHuman, Transport,
};
use klatch::harness::PsTable;
use klatch::member::{Call, CliClock, Who, custody, db_path, now, registry_and_domain};
use klatch::store::SqliteStore;
use klatch::store::areas::*;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use args::*;
use commands::*;

#[derive(Parser)]
#[command(name = "klatch", about = "Local agent rooms prototype", version = version_text(),
    after_help = "For a private or multi-message exchange with one member, run `klatch help dm`.")]
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

impl WhoFlags {
    fn who(&self) -> Who {
        Who {
            as_member: self.as_member.clone(),
            discover: self.discover,
            codex: self.codex,
            member_scope: self.member_scope.clone(),
            member_kind: self.member_kind.clone(),
        }
    }
}

/// What the command says about its caller's standard input (see `klatch::member::Call`).
fn call_of(cmd: &Cmd) -> Call {
    Call {
        hook: matches!(
            cmd,
            Cmd::Agents(AgentsCmd::Hook { .. }) | Cmd::Wait(WaitCmd::Wake { .. })
        ),
        session_end: matches!(cmd, Cmd::Agents(AgentsCmd::Hook { event: Some(e), .. }) if e == "SessionEnd"),
    }
}

#[derive(Subcommand)]
enum Cmd {
    #[command(flatten)]
    Messaging(MessagingCmd),
    #[command(flatten)]
    Wait(WaitCmd),
    #[command(flatten)]
    Work(WorkCmd),
    #[command(flatten)]
    Agents(AgentsCmd),
    /// Issue, revoke and list the credentials of workers (for a launcher; see `klatch worker --help`)
    #[command(display_order = 15)]
    Worker {
        #[command(subcommand)]
        cmd: worker::WorkerCmd,
    },
    /// Report a factory's work item done, or poll for finished ones, and release the claims anchored to them (for a
    /// launcher; see `klatch anchor --help`)
    #[command(display_order = 16)]
    Anchor {
        #[command(subcommand)]
        cmd: anchor::AnchorCmd,
    },
    /// Open and end a factory run, whose rooms are `run.<id>.<name>` (for a launcher; see `klatch run --help`)
    #[command(display_order = 17)]
    Run {
        #[command(subcommand)]
        cmd: run::RunCmd,
    },
    /// Controls for the person running klatch: pause, close, remove a member, audit (needs a real terminal)
    #[command(display_order = 18)]
    Admin {
        #[command(subcommand)]
        cmd: admin::AdminCmd,
    },
    /// Run the review queue through the judge (for a launcher; see `klatch judge --help`)
    #[command(display_order = 19)]
    Judge {
        #[command(subcommand)]
        cmd: judge::JudgeCmd,
    },
    /// Install klatch's hooks into Claude Code settings, after showing a diff and getting a yes per scope
    #[command(display_order = 34)]
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
        /// Keep a machine checkpoint of each session (off by default): a Stop hook, and a flag on the PostToolUse and
        /// SessionEnd hooks, record tool classes and edited paths (never a command, content or output) and write a
        /// compact checkpoint beside the member's task for a successor or a resume
        #[arg(long, conflicts_with = "no_checkpoint")]
        checkpoint: bool,
        /// Remove the machine checkpoint hook and the flag
        #[arg(long)]
        no_checkpoint: bool,
        /// Claim files as they are edited (off by default): a flag on the PostToolUse, SessionEnd and Stop hooks claims
        /// the repo-relative path of each file an edit or write touched, for 5 minutes renewed by each edit, releases
        /// it when the turn or session ends, and prints one line when another member holds the path (never a block)
        #[arg(long, conflicts_with = "no_passive_claims")]
        passive_claims: bool,
        /// Remove the passive claims flag
        #[arg(long)]
        no_passive_claims: bool,
        /// Remove every hook klatch installed (and nothing else) from the settings of --scope; keeps your rooms
        /// and keys
        #[arg(
            long,
            conflicts_with_all = ["board", "no_lobby", "no_session_end", "enforce", "no_enforce", "wake", "no_wake", "checkpoint", "no_checkpoint", "passive_claims", "no_passive_claims"]
        )]
        uninstall: bool,
    },
    /// Show the consents `klatch init` granted (board, join, session-end, enforce, wake, checkpoint, passive-claims), or switch one on or off
    /// later; edits only klatch's own hook entries, after a diff and a yes
    #[command(display_order = 35)]
    Consent {
        /// The consent to change: board, join, session-end, enforce, wake, checkpoint or passive-claims (none: show them all)
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
    #[command(display_order = 36)]
    Convention {
        /// The member name the child will use (letters, digits, - _ .); without one the text asks the child to pick
        #[arg(long)]
        name: Option<String>,
        /// The room the child works in (default lobby), named in every command of the brief
        #[arg(long)]
        room: Option<String>,
    },
    /// Check this installation: the binary, the store and its key, the hooks, the session (read only; creates nothing)
    #[command(display_order = 37)]
    Doctor,
    /// List the live coding-agent sessions on this machine from the harness registry (read only; never a key file)
    #[command(display_order = 38)]
    Sessions {
        /// Also list the records that are not live, with the reason, and how many files could not be read
        #[arg(long)]
        all: bool,
    },
    /// Write the event stream of a room to standard output as newline-delimited JSON (PROTOCOL.md)
    #[command(display_order = 46)]
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
    #[command(display_order = 47)]
    Serve {
        /// Use standard input and output (the only transport so far)
        #[arg(long)]
        stdio: bool,
        /// For an ssh forced command (`command="klatch serve --stdio --ssh-key LABEL",restrict ssh-ed25519 ...` in
        /// authorized_keys): the member is the one the human mapped LABEL to with `klatch admin ssh-key`, and no
        /// other. An unmapped label does not start; a typed name (--as-member, --discover, KLATCH_NAME) is never read
        #[arg(long, value_name = "LABEL")]
        ssh_key: Option<String>,
    },
    /// Run an MCP (Model Context Protocol) server on standard input and output as the member from the environment, so
    /// an agent runtime that can attach a tool server can post, read, wait, claim and release without a harness
    #[command(display_order = 48)]
    Mcp {
        /// Use standard input and output (the only transport so far)
        #[arg(long)]
        stdio: bool,
    },
    /// Offer the read-only librarian to an editor as an ACP (Agent Client Protocol) agent on standard input and output,
    /// as the member from the environment; it can only look (read without marking read, history, who, claims)
    #[command(display_order = 49)]
    Acp {
        /// Use standard input and output (the only transport so far)
        #[arg(long)]
        stdio: bool,
    },
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
    // A hook already runs with discovery on, so "pass --discover" would send the user in a circle: say what is
    // missing instead (#1313).
    let cause = e.root_cause().to_string().replace(
        "or pass --discover (or KLATCH_DISCOVER=1) to act as this coding session",
        "no session was found for this process in the harness registry",
    );
    let why: String = klatch::domain::flat(&cause).chars().take(120).collect();
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
        if is_closed_pipe(info.payload()) {
            // The reader went away (`klatch who lobby | head`): end quietly, as a shell program killed by SIGPIPE
            // does (128 + 13), instead of calling a normal ending a bug (#846).
            std::process::exit(141);
        }
        let at = info.location().map_or_else(
            || "unknown place".to_string(),
            |l| format!("{}:{}", l.file(), l.line()),
        );
        eprintln!("klatch: internal error at {at}; this is a bug (nothing you sent is shown here)");
    }));
    disable_core_dumps();
}

/// Whether a panic payload is the standard library's report that a print to a closed standard stream failed.
/// SIGPIPE stays ignored (the socket code needs the write error), so `println!` panics on a closed pipe.
fn is_closed_pipe(payload: &(dyn std::any::Any + Send)) -> bool {
    let text = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("");
    text.starts_with("failed printing to std") && text.contains("Broken pipe")
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
        Cmd::Agents(AgentsCmd::Hook { event, .. }) => Some(event.clone()),
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
            checkpoint,
            no_checkpoint,
            passive_claims,
            no_passive_claims,
            uninstall,
        } => run_init(&init::Options {
            scope,
            board,
            yes,
            no_lobby,
            no_session_end,
            enforce: init::Switch::from_flags(enforce, no_enforce),
            wake: init::Switch::from_flags(wake, no_wake),
            checkpoint: init::Switch::from_flags(checkpoint, no_checkpoint),
            passive_claims: init::Switch::from_flags(passive_claims, no_passive_claims),
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
        Cmd::Wait(WaitCmd::Wake { .. }) if !wake_is_on() => Ok(()),
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
    let Some((registry, domain)) = registry_and_domain() else {
        bail!("HOME is not set, so the harness registry cannot be found");
    };
    let found = klatch::harness::list(&registry, &PsTable, domain);
    print_live_sessions(&found);
    if all {
        print_ignored_sessions(&found);
    } else {
        note_ignored_sessions(&found);
    }
    Ok(())
}

/// The live sessions, one tab-separated line each, or a note on stderr when there are none.
fn print_live_sessions(found: &klatch::harness::Sessions) {
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
}

/// Why a record was ignored, in words.
fn liveness_reason(why: &klatch::harness::Liveness) -> &'static str {
    use klatch::harness::Liveness;
    match why {
        Liveness::Dead => "not running",
        Liveness::Reused => "pid reused by another process",
        Liveness::NotHarness => "running, but not the harness",
        Liveness::OtherDomain => "recorded on another machine",
        Liveness::Live => "live",
        Liveness::Unreadable => "could not be checked (the process table cannot be read)",
    }
}

/// `sessions --all`: every ignored record with its reason, then the unreadable count.
fn print_ignored_sessions(found: &klatch::harness::Sessions) {
    for (r, why) in &found.ignored {
        println!(
            "ignored\t{}\t{}\t{}\t{}",
            r.pid,
            liveness_reason(why),
            r.name.as_deref().unwrap_or("-"),
            r.cwd
        );
    }
    println!("unreadable\t{}", found.malformed);
}

/// `sessions` without `--all`: one stderr line counting what was left out, if anything was.
fn note_ignored_sessions(found: &klatch::harness::Sessions) {
    if !found.ignored.is_empty() || found.malformed > 0 {
        eprintln!(
            "{} other record(s) ignored (not running, a reused pid, not the harness or another machine) and {} unreadable; --all lists them",
            found.ignored.len(),
            found.malformed
        );
    }
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
    // an ssh forced command (#107): the member is the one the key label is mapped to, found before anything else
    if let Cmd::Serve {
        stdio,
        ssh_key: Some(label),
    } = &cmd
    {
        let member = klatch::member::open_ssh_member(&who.who(), label)?;
        return serve_stdio(
            &member.gateway,
            &member.caller.principal,
            (*stdio, Some(label.as_str())),
            false, // a remote ssh peer: nothing about this machine's process tree speaks for it
        );
    }
    let Some(member) = klatch::member::open_member(&who.who(), call_of(&cmd))? else {
        return Ok(());
    };
    let klatch::member::Member { gateway, caller } = member;
    match cmd {
        Cmd::Serve { stdio, .. } => serve_stdio(
            &gateway,
            &caller.principal,
            (stdio, None),
            caller.claude_code,
        ),
        Cmd::Mcp { stdio } => mcp_stdio(&gateway, &caller.principal, stdio, caller.claude_code),
        Cmd::Acp { stdio } => acp_stdio(&gateway, &caller.principal, stdio, caller.claude_code),
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

/// The harness a process the walk up the tree verified as Claude Code is judged as (#877, #1115, #1130); it is passed
/// only when the caller's process tree vouched for it, never because a request says so.
fn verified_harness() -> klatch::harness::ClaudeCode {
    klatch::harness::ClaudeCode::probing(klatch::harness::wake_support_here)
}

/// `klatch serve --stdio`; `(stdio, key)` where `key` is the label of the ssh key that fixed the member: a request that
/// claims an identity is refused (#107), and the session ends when the key stops being mapped to this member (#1083).
fn serve_stdio(
    gateway: &Gateway<SqliteStore>,
    principal: &Principal,
    (stdio, key): (bool, Option<&str>),
    claude_code: bool,
) -> Result<()> {
    if !stdio {
        bail!("serve needs --stdio, the only transport so far");
    }
    // stdout carries protocol frames only; nothing else may print there
    // stdin cannot be stopped from outside; the process exits when the session does
    let config = klatch::session::SessionConfig::default();
    let session = klatch::session::Session::new(gateway, principal, &config);
    // A `join` is judged by the harness the process tree verified, the same evidence a CLI command passes (#877, #1115)
    let harness = verified_harness();
    let session = if claude_code {
        session.with_harness(&harness)
    } else {
        session
    };
    // The mapping is read through a second connection of its own, opened here, so the look never goes through the
    // gateway's operations (it is a policy read for the session itself, #1083).
    let mapping = match key {
        Some(_) => Some(klatch::member::custody::open_store(
            &klatch::member::db_path()?,
        )?),
        None => None,
    };
    let still_mapped = || {
        ssh_key_still_mapped(
            mapping.as_ref().expect("an ssh session has a store"),
            key.unwrap_or_default(),
            &principal.member,
        )
    };
    let session = if key.is_some() {
        session.fixed_member().rechecked(&still_mapped)
    } else {
        session
    };
    Ok(session.serve_guarded(
        std::io::BufReader::new(std::io::stdin()),
        std::io::stdout(),
        None,
    )?)
}

/// Whether the ssh key `label` is still mapped to `member`: the refusal an open session ends with when the human took
/// the mapping away (`ssh-key-unmapped`) or pointed the label at another member (`ssh-key-remapped`).
fn ssh_key_still_mapped(
    store: &SqliteStore,
    label: &str,
    member: &str,
) -> klatch::domain::Result<()> {
    let (rule, what) = match klatch::gateway::ssh_key_member(store, label)? {
        Some(m) if m == member => return Ok(()),
        Some(_) => ("ssh-key-remapped", "is now mapped to another member"),
        None => ("ssh-key-unmapped", "is no longer mapped to a member"),
    };
    Err(klatch::domain::Error::Refused {
        rule: rule.into(),
        reason: format!(
            "the ssh key {label} {what}; this session ends, and a new one starts as whoever the key is mapped to"
        ),
    })
}

/// `klatch mcp --stdio`: the MCP front end over the same session `serve` uses; standard output carries MCP messages only.
fn mcp_stdio(
    gateway: &Gateway<SqliteStore>,
    principal: &Principal,
    stdio: bool,
    claude_code: bool,
) -> Result<()> {
    if !stdio {
        bail!("mcp needs --stdio, the only transport so far");
    }
    let config = klatch::session::SessionConfig::default();
    let session = klatch::session::Session::new(gateway, principal, &config);
    // A `klatch_join` is judged by the harness the process tree verified, as for `serve` (#1130)
    let harness = verified_harness();
    let session = if claude_code {
        session.with_harness(&harness)
    } else {
        session
    };
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
fn acp_stdio(
    gateway: &Gateway<SqliteStore>,
    principal: &Principal,
    stdio: bool,
    claude_code: bool,
) -> Result<()> {
    if !stdio {
        bail!("acp needs --stdio, the only transport so far");
    }
    let config = klatch::session::SessionConfig::default();
    let session = klatch::session::Session::new(gateway, principal, &config);
    // The same evidence as for `serve` (#1130); the ACP agent only looks today, so no join reaches it
    let harness = verified_harness();
    let session = if claude_code {
        session.with_harness(&harness)
    } else {
        session
    };
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

#[cfg(test)]
mod hook_notice_tests {
    #[test]
    fn a_hook_that_already_discovers_is_never_told_to_pass_discover() {
        let e = anyhow::Error::new(klatch::domain::Error::Invalid(
            "no member name given: set KLATCH_NAME (or --as-member) to 1-40 chars of [A-Za-z0-9._-], \
             or pass --discover (or KLATCH_DISCOVER=1) to act as this coding session"
                .into(),
        ));
        for event in [
            "UserPromptSubmit",
            "PostToolUse",
            "PreToolUse",
            "SessionStart",
        ] {
            let line = super::failure_line(&e, event);
            assert!(!line.contains("--discover"), "{event}: {line}");
            assert!(line.contains("no session"), "{event}: {line}");
        }
    }
}
