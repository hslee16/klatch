//! klatch: local prototype of the agent rooms gateway (DESIGN.md M1 thin slice).
//!
//! Identity is the KLATCH_NAME env var, which is FORGEABLE (prototype only; see DESIGN.md 8.4).
//! All state goes through the `Store` trait; this file only parses arguments and prints.

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use klatch_core::{RoomDigest, flat, render_hook, valid_name};
use klatch_store::{SqliteStore, Store};
use std::path::PathBuf;
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
    run(&store, &me, cli.cmd, now())
}

/// Dispatch one command against any store. Separate from `main` so it can be tested without a process.
fn run(store: &dyn Store, me: &str, cmd: Cmd, now: i64) -> Result<()> {
    match cmd {
        Cmd::Join { room } => {
            if !valid_name(&room) {
                bail!("bad room name");
            }
            store.join(&room, me, now)?;
            println!("{me} joined {room}");
        }
        Cmd::Post { room, body } => {
            store.post(&room, me, &body.join(" "), now)?;
            println!("posted (queued; not proof of read)");
        }
        Cmd::Read { room, peek } => {
            for m in store.read(&room, me, !peek, now)? {
                println!("[{} {} {}] {}", m.id, m.ts, m.sender, flat(&m.body));
            }
        }
        Cmd::Who { room } => {
            for m in store.members(&room)? {
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
            store.claim(&room, me, &path, ttl, &reason, now)?;
            println!("claimed {path} for {ttl}s (advisory)");
        }
        Cmd::Release { room, path } => {
            store.release(&room, me, &path)?;
            println!("released {path}");
        }
        Cmd::Claims { room } => {
            for c in store.claims(&room, now)? {
                println!(
                    "{}\t{}\t{}s left\t{}",
                    c.path,
                    c.owner,
                    c.expires - now,
                    flat(&c.reason)
                );
            }
        }
        Cmd::Hook => {
            let mut digests = Vec::new();
            for room in store.rooms_of(me)? {
                let messages = store.read(&room, me, true, now)?;
                let claims = store
                    .claims(&room, now)?
                    .into_iter()
                    .filter(|c| c.owner != me)
                    .collect();
                digests.push(RoomDigest {
                    room,
                    messages,
                    claims,
                });
            }
            print!("{}", render_hook(&digests, now));
        }
    }
    Ok(())
}
