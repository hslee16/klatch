//! klatch: local prototype of the agent rooms gateway (DESIGN.md M1 thin slice).
//!
//! Identity is the KLATCH_NAME env var, which is FORGEABLE (prototype only; see DESIGN.md 8.4).
//! All state goes through the `Store` trait; this file only parses arguments and prints.

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use klatch_core::{Principal, flat, render_hook, valid_name};
use klatch_gateway::{AllowAll, Direct, Gateway, SystemClock, Transport};
use klatch_store::SqliteStore;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Parser)]
#[command(name = "klatch", about = "Local agent rooms prototype")]
struct Cli {
    /// Member name (default: $KLATCH_NAME)
    #[arg(long, global = true)]
    as_member: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
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
    /// List members of a room with last activity
    Who { room: String },
    /// Claim a path prefix (advisory lease with TTL)
    Claim {
        room: String,
        path: String,
        #[arg(long, default_value_t = 600)]
        ttl: i64,
        #[arg(long, default_value = "")]
        reason: String,
    },
    /// Release a claim you hold
    Release { room: String, path: String },
    /// List active claims in a room
    Claims { room: String },
    /// Hook output for UserPromptSubmit/SessionStart: unread messages and claims across joined rooms
    Hook,
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

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn db_path() -> Result<PathBuf> {
    let base = match std::env::var("KLATCH_HOME") {
        Ok(p) => PathBuf::from(p),
        Err(_) => PathBuf::from(std::env::var("HOME").context("HOME not set")?).join(".klatch"),
    };
    std::fs::create_dir_all(&base)?;
    Ok(base.join("rooms.db"))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let me = cli
        .as_member
        .or_else(|| std::env::var("KLATCH_NAME").ok())
        .unwrap_or_default();
    if !valid_name(&me) {
        bail!("set KLATCH_NAME (or --as-member) to 1-40 chars of [A-Za-z0-9._-]");
    }
    let store = SqliteStore::open(&db_path()?)?;
    let gateway = Rc::new(Gateway::new(store, SystemClock, AllowAll));
    // The name comes from an environment variable, so it is not proven: the caller is unattributed.
    let principal = Principal::unattributed(&me);
    if let Cmd::Serve { stdio } = cli.cmd {
        if !stdio {
            bail!("serve needs --stdio, the only transport so far");
        }
        // stdout carries protocol frames only; nothing else may print there
        return Ok(klatch_session::serve(
            &gateway,
            &principal,
            std::io::BufReader::new(std::io::stdin()),
            std::io::stdout().lock(),
        )?);
    }
    if let Cmd::Stream {
        room,
        from_seq,
        follow,
        poll_ms,
    } = &cli.cmd
    {
        let config = klatch_session::SessionConfig {
            poll: std::time::Duration::from_millis(*poll_ms),
            ..Default::default()
        };
        // stdout carries protocol frames only
        return Ok(klatch_session::stream(
            &gateway,
            &principal,
            room,
            *from_seq,
            *follow,
            std::io::stdout().lock(),
            &config,
        )?);
    }
    let transport = Direct::new(gateway, principal);
    run(&transport, &me, cli.cmd, now())
}

/// Dispatch one command through any transport. `now` is the client clock, used only for display.
fn run(t: &dyn Transport, me: &str, cmd: Cmd, now: i64) -> Result<()> {
    match cmd {
        Cmd::Join { room } => {
            t.join(&room)?;
            println!("{me} joined {room}");
        }
        Cmd::Post { room, body } => {
            t.post(&room, &body.join(" "))?;
            println!("posted (queued; not proof of read)");
        }
        Cmd::Read { room, peek } => {
            for m in t.read(&room, !peek)? {
                println!("[{} {} {}] {}", m.seq, m.ts, m.sender, flat(&m.body));
            }
        }
        Cmd::Who { room } => {
            for m in t.who(&room)? {
                println!(
                    "{}\tlast seen {}s ago (liveness, not progress)",
                    m.name,
                    now - m.last_seen
                );
            }
        }
        Cmd::Claim {
            room,
            path,
            ttl,
            reason,
        } => {
            t.claim(&room, &path, ttl, &reason)?;
            println!("claimed {path} for {ttl}s (advisory)");
        }
        Cmd::Release { room, path } => {
            t.release(&room, &path)?;
            println!("released {path}");
        }
        Cmd::Claims { room } => {
            for c in t.claims(&room)? {
                println!(
                    "{}\t{}\t{}s left\t{}",
                    c.path,
                    c.owner,
                    c.expires - now,
                    flat(&c.reason)
                );
            }
        }
        Cmd::Serve { .. } | Cmd::Stream { .. } => {
            unreachable!("serve and stream are handled before dispatch")
        }
        Cmd::Hook => {
            let d = t.digest()?;
            for r in &d.refused {
                println!("[klatch] room {} not shown: {}", r.room, flat(&r.reason));
            }
            print!("{}", render_hook(&d.rooms, d.now));
        }
    }
    Ok(())
}
