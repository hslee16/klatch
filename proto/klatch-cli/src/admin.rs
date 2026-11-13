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
        AdminCmd::Remove { member, .. } => Some(member.clone()),
        _ => None,
    }
}

fn typed_back(expected: &str, line: &str) -> bool {
    line.trim() == expected
}

pub fn run(cmd: AdminCmd) -> Result<()> {
    let terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    // a test build can stand in for the terminal; a normal build never does
    #[cfg(feature = "test-clock")]
    let terminal = terminal || std::env::var("KLATCH_ADMIN_TEST_TERMINAL").is_ok_and(|v| v == "1");
    let agent_above = discover_session().is_ok();
    if let Err(why) = gate(terminal, agent_above) {
        bail!("{why}");
    }
    if let Some(expected) = confirmation(&cmd) {
        print!("type {expected} to confirm: ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        if !typed_back(&expected, &line) {
            bail!("not confirmed; nothing was changed");
        }
    }
    let store = SqliteStore::open(&db_path()?)?;
    let gateway = Rc::new(Gateway::new(store, CliClock, AllowAll));
    let t = Direct::new(gateway, Principal::unattributed("admin"));
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
            let room = if all_rooms {
                "*".to_string()
            } else {
                room.unwrap_or_default()
            };
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
            let room = if all_rooms {
                "*".to_string()
            } else {
                room.unwrap_or_default()
            };
            t.admin_allow_member(&room, &member)?;
            println!("{member} is allowed in {room} again");
        }
        AdminCmd::Audit { limit } => {
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
        }
    }
    Ok(())
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
