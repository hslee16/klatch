//! `klatch admin ...`: the controls a person uses to stop things (R11, ticket #23): close or reopen a room, pause or
//! resume every room, remove a member from a room (or from all rooms) and allow it back, and read the audit.
//!
//! Who may administer is decided here, not in the gateway: a command must run at a real terminal (agents run
//! commands with none), with no coding-agent session above the process, and the destructive ones ask for the target
//! typed back. That is a heuristic, not proof: an agent running as the same user could open a pseudo-terminal.
//! Real proof needs roles and authentication (epic #10, #152), at which point only `gate` changes.

use super::*;
use clap::Subcommand;
use std::io::{BufRead, IsTerminal, Write};

#[derive(Subcommand)]
pub enum AdminCmd {
    /// Stop every room: nothing is posted, claimed or delivered until `admin resume`
    Pause,
    /// Start every room again
    Resume,
    /// Make a room read-only (history stays readable)
    Close { room: String },
    /// Open a closed room again
    Reopen { room: String },
    /// Take a member out of a room, release its claims there and refuse it there until `admin allow`
    Remove {
        member: String,
        /// The room to remove it from
        #[arg(
            long,
            conflicts_with = "all_rooms",
            required_unless_present = "all_rooms"
        )]
        room: Option<String>,
        /// Every room, not one
        #[arg(long)]
        all_rooms: bool,
    },
    /// Lift a removal
    Allow {
        member: String,
        #[arg(
            long,
            conflicts_with = "all_rooms",
            required_unless_present = "all_rooms"
        )]
        room: Option<String>,
        #[arg(long)]
        all_rooms: bool,
    },
    /// Set a room's label (public, internal, private, secret by default); members below it are refused in it
    Label { room: String, label: String },
    /// Set a member's clearance, at most the agent cap
    Clear { member: String, label: String },
    /// Set the highest clearance any agent may hold; it must be below the top level
    Cap { label: String },
    /// Replace the label order with a comma separated chain, lowest first, for example open,team,board
    Chain { levels: String },
    /// Plant a canary (a fake credential) in a room; text carrying it into any other room is refused and pauses
    /// that room until `admin reopen`
    Canary { room: String },
    /// Refuse posts, claim reasons, release notes and statuses that contain a private term, in rooms below the
    /// agent cap; matching ignores case and a refusal never repeats the term
    ScrubAdd { name: String, term: Vec<String> },
    /// Stop refusing a scrub rule
    ScrubRemove { name: String },
    /// Show the scrub rules and their terms
    ScrubList,
    /// Show the recovery code of this store's encryption key (audited). Keep it somewhere safe: with it the key can
    /// be restored, and without both it and the keystore entry the data is gone
    Recovery,
    /// Restore this store's key from its recovery code, when the keystore entry is missing
    Recover {
        code: Vec<String>,
        /// The code is for this sealed label's key, not the store's
        #[arg(long)]
        label: Option<String>,
    },
    /// Seal a label: its messages are stored encrypted under a key of their own, so only a principal holding that key
    /// (its clearance reaches the label and the key was handed to it) can read or write them
    SealLabel { label: String },
    /// Re-encrypt the store under a new key; the old key (and the old recovery code) stop working
    RotateKey,
    /// Write an encrypted copy of the whole store to a file (never overwrites one); keep it with its recovery code
    Backup { file: std::path::PathBuf },
    /// Replace the store with a backup made under the same key; the store it replaces is kept beside it
    Restore { file: std::path::PathBuf },
    /// Print a room's messages as readable text. This is the one way plaintext leaves the store on purpose, so it is
    /// audited and asks for the room typed back
    Export { room: String },
    /// Show the latest refused calls: who, where, which operation and the rule, never the text
    Refusals {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Show the latest administrative acts, newest first
    Audit {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
}

/// Why an admin command may not run here, or `Ok`. `terminal` is a real terminal on standard input and output;
/// `agent_above` is a coding-agent session above this process.
pub fn gate(terminal: bool, agent_above: bool) -> Result<(), &'static str> {
    if agent_above {
        return Err(
            "admin commands cannot run inside a coding-agent session; run them from a terminal of your own",
        );
    }
    if !terminal {
        return Err("admin commands need a real terminal; run them yourself in a terminal");
    }
    Ok(())
}

/// What must be typed back to confirm, if the command is destructive.
fn confirmation(cmd: &AdminCmd) -> Option<String> {
    match cmd {
        AdminCmd::Pause => Some("pause".into()),
        AdminCmd::Close { room } => Some(room.clone()),
        AdminCmd::Recover { .. } => Some("recover".into()),
        AdminCmd::RotateKey => Some("rotate".into()),
        AdminCmd::Restore { .. } => Some("restore".into()),
        AdminCmd::Export { room } => Some(room.clone()),
        AdminCmd::Remove { member, .. } => Some(member.clone()),
        _ => None,
    }
}

fn typed_back(expected: &str, line: &str) -> bool {
    line.trim() == expected
}

/// The room an admin act is about: every room (`*`) when asked, else the one named.
fn room_of(room: Option<String>, all_rooms: bool) -> String {
    if all_rooms {
        "*".to_string()
    } else {
        room.unwrap_or_default()
    }
}

/// Refuse unless this is a person at a terminal, and ask for the target typed back when the act is destructive.
fn authorize(cmd: &AdminCmd) -> Result<()> {
    let terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    // a test build can stand in for the terminal; a normal build never does
    #[cfg(feature = "test-clock")]
    let terminal = terminal || std::env::var("KLATCH_ADMIN_TEST_TERMINAL").is_ok_and(|v| v == "1");
    if let Err(why) = gate(terminal, discover_session().is_ok()) {
        bail!("{why}");
    }
    let Some(expected) = confirmation(cmd) else {
        return Ok(());
    };
    print!("type {expected} to confirm: ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    if !typed_back(&expected, &line) {
        bail!("not confirmed; nothing was changed");
    }
    Ok(())
}

fn audit(t: &dyn Transport, limit: usize) -> Result<()> {
    let now = now();
    for a in t.admin_audit(limit)? {
        println!(
            "{}s ago\t{}\t{}\t{}",
            klatch_core::secs_between(now, a.ts),
            a.action,
            flat(&a.target),
            flat(&a.detail)
        );
    }
    Ok(())
}

fn refusals(t: &dyn Transport, limit: usize) -> Result<()> {
    let now = now();
    for r in t.admin_refusals(limit)? {
        println!(
            "{}s ago\t{}\t{}\t{}\t{}",
            klatch_core::secs_between(now, r.ts),
            flat(&r.member),
            flat(&r.room),
            r.operation,
            r.rule
        );
    }
    Ok(())
}

fn execute(t: &dyn Transport, cmd: AdminCmd) -> Result<()> {
    match cmd {
        AdminCmd::Pause => {
            t.admin_pause(true)?;
            println!(
                "all rooms are paused; nothing is posted, claimed or delivered until `klatch admin resume`"
            );
        }
        AdminCmd::Resume => {
            t.admin_pause(false)?;
            println!("rooms resumed");
        }
        AdminCmd::Close { room } => {
            t.admin_close_room(&room, true)?;
            println!("room {room} is closed (read-only)");
        }
        AdminCmd::Reopen { room } => {
            t.admin_close_room(&room, false)?;
            println!("room {room} is open again");
        }
        AdminCmd::Remove {
            member,
            room,
            all_rooms,
        } => {
            let room = room_of(room, all_rooms);
            t.admin_remove_member(&room, &member)?;
            println!(
                "{member} is removed from {room} and refused there until `klatch admin allow`"
            );
        }
        AdminCmd::Allow {
            member,
            room,
            all_rooms,
        } => {
            let room = room_of(room, all_rooms);
            t.admin_allow_member(&room, &member)?;
            println!("{member} is allowed in {room} again");
        }
        AdminCmd::Label { room, label } => {
            t.admin_set_room_label(&room, &label)?;
            println!("room {room} is labelled {label}");
        }
        AdminCmd::Clear { member, label } => {
            t.admin_set_clearance(&member, &label)?;
            println!("{member} is cleared to {label}");
        }
        AdminCmd::Cap { label } => {
            t.admin_set_cap(&label)?;
            println!("the agent cap is {label}");
        }
        AdminCmd::Chain { levels } => {
            let names: Vec<&str> = levels.split(',').map(str::trim).collect();
            t.admin_set_label_chain(&names)?;
            println!(
                "label order is {}; the agent cap is back to its default",
                names.join(" < ")
            );
        }
        AdminCmd::Canary { room } => {
            t.admin_plant_canary(&room)?;
            println!("a canary is planted in {room}; a copy of it anywhere else pauses that room");
        }
        AdminCmd::ScrubAdd { name, term } => {
            t.admin_add_scrub_term(&name, &term.join(" "))?;
            println!("rule {name} now refuses its term in rooms below the agent cap");
        }
        AdminCmd::ScrubRemove { name } => {
            t.admin_remove_scrub_term(&name)?;
            println!("rule {name} removed");
        }
        AdminCmd::ScrubList => {
            for (name, term) in t.admin_scrub_terms()? {
                println!("{name}\t{}", flat(&term));
            }
        }
        AdminCmd::Refusals { limit } => refusals(t, limit)?,
        AdminCmd::Audit { limit } => audit(t, limit)?,
        AdminCmd::Recovery
        | AdminCmd::Recover { .. }
        | AdminCmd::RotateKey
        | AdminCmd::Backup { .. }
        | AdminCmd::Restore { .. }
        | AdminCmd::Export { .. }
        | AdminCmd::SealLabel { .. } => {
            unreachable!("handled in run, which needs the keystore and not only the gateway")
        }
    }
    Ok(())
}

/// The commands about the key: they need the keystore as well as the gateway, and each one is in the audit.
fn key_command(db: &std::path::Path, cmd: AdminCmd) -> Result<()> {
    let who = Principal::unattributed("admin");
    let note = |action: &str, outcome: &str| -> Result<()> {
        let gateway = Gateway::new(keys::open_store(db)?, CliClock, AllowAll);
        Ok(gateway.admin_note(&who, (action, ""), outcome)?)
    };
    match cmd {
        AdminCmd::Recovery => {
            let key = keys::current_key(db)?.ok_or_else(|| {
                anyhow::anyhow!("this store has no key yet; run any klatch command to make one")
            })?;
            note("recovery-show", "")?;
            println!("{}", klatch_keys::recovery_code(&key));
            let gateway = Gateway::new(keys::open_store(db)?, CliClock, AllowAll);
            for label in gateway.sealed_labels()? {
                if let Some(k) = keys::label_key(db, &label)? {
                    println!("label {label}: {}", klatch_keys::recovery_code(&k));
                }
            }
            println!(
                "Keep these codes somewhere safe and private. Each restores one key; they are not stored anywhere else."
            );
        }
        AdminCmd::Recover { code, label } => {
            let key = klatch_keys::restore(&code.join(" ")).map_err(|e| anyhow::anyhow!("{e}"))?;
            match &label {
                Some(l) => keys::restore_label_key(db, l, &key)?,
                None => keys::restore_key(db, &key)?,
            }
            note("recover-key", label.as_deref().unwrap_or(""))?;
            println!("the key is back in place");
        }
        AdminCmd::SealLabel { label } => {
            let gateway = Gateway::new(keys::open_store(db)?, CliClock, AllowAll);
            gateway.admin_seal_label(&who, &label)?;
            let made = keys::ensure_label_key(db, &label)?;
            println!(
                "label {label} is sealed: new messages under it are stored encrypted under a key of their own"
            );
            if made {
                println!(
                    "a key was made for it; `klatch admin recovery` shows the code that restores it. Keep that code: a sealed label whose key is lost cannot be read again."
                );
            }
        }
        AdminCmd::Backup { file } => {
            let key = keys::current_key(db)?
                .ok_or_else(|| anyhow::anyhow!("this store has no key yet; nothing to back up"))?;
            keys::open_store(db)?.backup_to(&file, &key)?;
            note("backup", &file.to_string_lossy())?;
            println!(
                "wrote an encrypted backup to {} (it opens only with this store's key: keep the recovery code too)",
                file.display()
            );
        }
        AdminCmd::Restore { file } => {
            let key = keys::current_key(db)?.ok_or_else(|| {
                anyhow::anyhow!(
                    "this store has no key; restore the key first with `klatch admin recover`"
                )
            })?;
            klatch_store::encrypt::restore_backup(db, &file, &key)?;
            note("restore", &file.to_string_lossy())?;
            println!(
                "restored {}; the store it replaced is kept as rooms.db.before-restore beside it",
                file.display()
            );
        }
        AdminCmd::Export { room } => {
            let gateway = Gateway::new(keys::open_store(db)?, CliClock, AllowAll);
            gateway.use_label_keys(keys::label_ring(db, &gateway.sealed_labels()?)?);
            // audited before anything is printed: the act is recorded even if the output is cut short
            gateway.admin_note(&who, ("export", &room), "readable transcript printed")?;
            for m in gateway.tail(&who, &room, (0, 1_000_000))? {
                println!(
                    "[{} {} {} {}] {}",
                    m.seq,
                    m.ts,
                    m.sender_label(),
                    m.label,
                    flat(&m.body)
                );
            }
        }
        AdminCmd::RotateKey => {
            let key = keys::rotate_key(db)?;
            note("rotate-key", "")?;
            println!(
                "the store is now encrypted under a new key. The old key and the old recovery code no longer work."
            );
            println!("New recovery code: {}", klatch_keys::recovery_code(&key));
        }
        _ => unreachable!("not a key command"),
    }
    Ok(())
}

/// The commands that act on the key or the store file itself, and so need more than the gateway.
fn needs_the_keystore(cmd: &AdminCmd) -> bool {
    matches!(
        cmd,
        AdminCmd::Recovery
            | AdminCmd::Recover { .. }
            | AdminCmd::RotateKey
            | AdminCmd::Backup { .. }
            | AdminCmd::Restore { .. }
            | AdminCmd::Export { .. }
            | AdminCmd::SealLabel { .. }
    )
}

pub fn run(cmd: AdminCmd) -> Result<()> {
    authorize(&cmd)?;
    let db = db_path()?;
    if needs_the_keystore(&cmd) {
        return key_command(&db, cmd);
    }
    let store = keys::open_store(&db)?;
    let gateway = Rc::new(Gateway::new(store, CliClock, AllowAll));
    // the person administering holds every label key
    gateway.use_label_keys(keys::label_ring(&db, &gateway.sealed_labels()?)?);
    execute(&Direct::new(gateway, Principal::unattributed("admin")), cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_real_terminal_with_no_agent_session_above_may_administer() {
        assert!(gate(true, false).is_ok());
        assert!(gate(false, false).unwrap_err().contains("real terminal"));
        assert!(
            gate(true, true)
                .unwrap_err()
                .contains("coding-agent session")
        );
        assert!(gate(false, true).is_err());
    }

    #[test]
    fn the_destructive_commands_ask_for_the_target_typed_back() {
        assert_eq!(confirmation(&AdminCmd::Pause).as_deref(), Some("pause"));
        assert_eq!(
            confirmation(&AdminCmd::Close {
                room: "lobby".into()
            })
            .as_deref(),
            Some("lobby")
        );
        assert!(confirmation(&AdminCmd::Resume).is_none());
        assert!(confirmation(&AdminCmd::Audit { limit: 5 }).is_none());
        assert!(typed_back("lobby", "lobby\n"));
        assert!(!typed_back("lobby", "lobb\n"));
        assert!(!typed_back("lobby", "\n"));
    }
}
