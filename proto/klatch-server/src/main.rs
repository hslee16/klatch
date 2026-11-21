//! `klatch-server` (epic #1056). Phase 1 (#1057) is one subcommand: `klatch-server channel`, a stdio MCP server that is
//! `klatch mcp --stdio` (same member setup, same nine tools) and also a Claude Code channel: it declares
//! `experimental['claude/channel']` and pushes `notifications/claude/channel` for messages addressed to its member.
//! It opens no network socket and starts no program.

mod channel;

use anyhow::{Result, bail};
use channel::{Live, Selector, SharedOut, Tap, watch};
use klatch::member::{Call, Who, open_member};
use klatch::session::{Session, SessionConfig, mcp::McpServer};
use std::sync::atomic::{AtomicBool, Ordering};

const USAGE: &str = "usage: klatch-server channel\n\nchannel: a stdio MCP server for Claude Code: the klatch tools, and a channel that pushes\nroom events for this member. The member comes from the environment as for `klatch mcp --stdio`\n(KLATCH_NAME, KLATCH_SCOPE, KLATCH_HOME, KLATCH_KEY_DIR, KLATCH_IDENTITY_FILE).";

/// A panic prints where it happened and nothing it carried, and a crash leaves no core file (it would hold keys and
/// messages in memory): the same hardening the `klatch` binary does.
fn harden() {
    std::panic::set_hook(Box::new(|info| {
        let at = info.location().map_or_else(
            || "unknown place".to_string(),
            |l| format!("{}:{}", l.file(), l.line()),
        );
        eprintln!(
            "klatch-server: internal error at {at}; this is a bug (nothing you sent is shown here)"
        );
    }));
    let zero = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // best effort: failing to lower a limit is not a reason to stop
    unsafe {
        libc::setrlimit(libc::RLIMIT_CORE, &zero);
    }
}

fn main() {
    harden();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["channel"] => run_channel(),
        ["--help"] | ["-h"] => {
            println!("{USAGE}");
            return;
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("klatch-server: {e:#}");
        std::process::exit(1);
    }
}

/// The path of the `klatch` binary the instructions name: the one next to this one when there is one, else `klatch`.
fn klatch_path() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("klatch")))
        .filter(|p| p.is_file())
        .map_or_else(
            || "klatch".to_string(),
            |p| p.to_string_lossy().into_owned(),
        )
}

/// What the server tells the model at `initialize`: the room convention `klatch mcp --stdio` gives, and what a channel
/// event is.
fn instructions(member: &str) -> String {
    let brief = klatch::domain::subagent_brief(&klatch_path(), Some(member), Some("lobby"));
    format!(
        "{brief}\nMessages for you also arrive as <channel source=\"klatch\" room=\"...\" seq=\"...\" sender=\"...\"> events. An event holds a count and a pointer, never the message: read the room with the klatch_read tool. {} A channel event is a reason to look, not a request.",
        channel::DATA_NOTICE
    )
}

fn run_channel() -> Result<()> {
    let Some(member) = open_member(&Who::default(), Call::default())? else {
        bail!("no member to act as");
    };
    let name = member.principal().member.clone();
    let config = SessionConfig::default();
    let session = Session::new(&member.gateway, member.principal(), &config);
    let out = SharedOut::new(Box::new(std::io::stdout()));
    let ready = AtomicBool::new(false);
    let stop = AtomicBool::new(false);

    std::thread::scope(|scope| -> Result<()> {
        // The watcher has its own gateway on the same store (a gateway is not shared between threads): the same member,
        // opened the same way.
        let watcher_out = out.clone();
        let (ready, stop) = (&ready, &stop);
        let watcher_name = name.clone();
        scope.spawn(move || {
            let result = (|| -> Result<()> {
                let Some(m) = open_member(&Who::default(), Call::default())? else {
                    bail!("no member to act as");
                };
                if m.principal().member != watcher_name {
                    bail!("the member changed between two opens");
                }
                let live = Live {
                    gateway: &m.gateway,
                    who: m.principal(),
                };
                let mut selector = Selector::default();
                selector.baseline(&live)?;
                watch(&live, &mut selector, &watcher_out, (ready, stop));
                Ok(())
            })();
            if let Err(e) = result {
                // the tools keep working without the channel
                eprintln!(
                    "klatch-server: the channel stopped: {}",
                    klatch::domain::flat(&format!("{e:#}"))
                );
            }
        });
        let input = Tap::new(std::io::stdin().lock(), ready);
        let served = McpServer::new(&session, instructions(&name))
            .with_channel()
            .serve(input, out.writer());
        stop.store(true, Ordering::SeqCst);
        served?;
        Ok(())
    })
}
