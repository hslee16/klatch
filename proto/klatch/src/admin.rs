//! `klatch admin ...`: the controls a person uses to stop things (R11, ticket #23): close or reopen a room, pause or
//! resume every room, remove a member from a room (or from all rooms) and allow it back, and read the audit.
//!
//! Who may administer is decided here, not in the gateway: a command must run at a real terminal (agents run
//! commands with none), with no coding-agent session above the process, and the destructive ones ask for the target
//! typed back. That is a heuristic, not proof: an agent running as the same user could open a pseudo-terminal.
//! Real proof needs roles and authentication (epic #10, #152), at which point only `gate` changes.

use super::*;
use clap::{Subcommand, ValueEnum};
use std::io::{BufRead, IsTerminal, Write};

/// Who may create rooms and invite (#188).
#[derive(Clone, Copy, ValueEnum)]
pub enum RoomPolicy {
    Open,
    Granted,
}

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
    /// Make a member a read-only librarian (the interface for asking what happened), or with --off an ordinary member
    Librarian {
        member: String,
        #[arg(long)]
        off: bool,
    },
    /// Make a member a quarantined reviewer (reads queued items and records verdicts, nothing else), or with --off an ordinary member
    Reviewer {
        member: String,
        #[arg(long)]
        off: bool,
    },
    /// Make a member the admin agent: it may do the non-floor admin acts (rooms, members, clearances inside the cap,
    /// room labels, upkeep) and is refused and audited on anything else, floors included; --off removes the role
    AdminAgent {
        member: String,
        #[arg(long)]
        off: bool,
    },
    /// Require a grant to create rooms and invite (`granted`), or leave it to every member (`open`, the default)
    RoomCreation { policy: RoomPolicy },
    /// Give a member the capability to create rooms and invite, or with --off take it away (matters only under
    /// `room-creation granted`)
    GrantRooms {
        member: String,
        #[arg(long)]
        off: bool,
    },
    /// Limit every member to POSTS posts in SECS seconds over all rooms together, for example `rate-limit 30 60`;
    /// `rate-limit --off` lifts it. Off by default
    RateLimit {
        posts: Option<usize>,
        secs: Option<i64>,
        #[arg(long)]
        off: bool,
    },
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
    /// Print a run's archive (made by `klatch run end --archive`) as readable JSON lines. Plaintext leaves on purpose,
    /// so it is audited and asks for the run id typed back
    RunArchive {
        file: std::path::PathBuf,
        /// The run the archive is of
        #[arg(long)]
        run: String,
    },
    /// Keep a room's messages (`*` for every room without its own setting) for this many days before `admin archive`
    /// moves them out; `off` removes the setting. Nothing is archived for a room with no retention
    Retention { room: String, days: String },
    /// Move messages older than their room's retention into an encrypted archive file under the klatch directory,
    /// leaving a tombstone for each (the seq stays). Run it from cron to make it regular; there is no background process
    Archive {
        #[arg(
            long,
            conflicts_with = "all_rooms",
            required_unless_present = "all_rooms"
        )]
        room: Option<String>,
        #[arg(long)]
        all_rooms: bool,
    },
    /// Put archived messages back from an archive file (made by `admin archive`); the file is kept
    RestoreArchive { file: std::path::PathBuf },
    /// List the archive files under the klatch directory with their rooms, message counts, ages and size, each opened
    /// with the store key and checked for damage. Prints counts, never text; exits with an error if any file is unusable
    Archives,
    /// Print a room's messages as readable text. This is the one way plaintext leaves the store on purpose, so it is
    /// audited and asks for the room typed back
    Export {
        room: String,
        /// One JSON object per line, with every field, instead of the readable text
        #[arg(long)]
        json: bool,
    },
    /// Show the latest refused calls: who, where, which operation and the rule, never the text
    Refusals {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// List the review queue: crossings waiting, and what each verdict did (pointers and outcomes, never text)
    ReviewQueue {
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
        AdminCmd::Export { room, .. } => Some(room.clone()),
        AdminCmd::RunArchive { run, .. } => Some(run.clone()),
        AdminCmd::Remove { member, .. } => Some(member.clone()),
        _ => None,
    }
}

/// One message as a line of JSON: the derived, regenerable transcript form (#71). Every field the store keeps.
fn export_json_line(room: &str, m: &klatch::domain::Message) -> String {
    serde_json::json!({
        "room": room,
        "id": m.id,
        "seq": m.seq,
        "sender": m.sender,
        "attribution": match m.attribution {
            klatch::domain::Attribution::Verified => "verified",
            klatch::domain::Attribution::Unattributed => "unattributed",
        },
        "label": m.label,
        "ts": m.ts,
        "body": m.body,
        "kind": m.kind,
        "payload": m.payload,
    })
    .to_string()
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
            klatch::domain::secs_between(now, a.ts),
            a.action,
            flat(&a.target),
            flat(&a.detail)
        );
    }
    Ok(())
}

fn review_queue(t: &dyn Transport, limit: usize) -> Result<()> {
    let now = now();
    for r in t.review_queue(limit)? {
        let state = if r.verdict.is_empty() {
            "waiting"
        } else {
            &r.verdict
        };
        let applied = if r.applied.is_empty() {
            "-"
        } else {
            &r.applied
        };
        println!(
            "{}\t{}s ago\t{}#{}\t{}\t{}\t{}\t{}",
            r.id,
            klatch::domain::secs_between(now, r.ts),
            flat(&r.room),
            r.seq,
            if r.to_room.is_empty() {
                r.to_label.as_str()
            } else {
                r.to_room.as_str()
            },
            state,
            applied,
            flat(&r.requester)
        );
    }
    Ok(())
}

fn refusals(t: &dyn Transport, limit: usize) -> Result<()> {
    let now = now();
    for r in t.admin_refusals(limit)? {
        println!(
            "{}s ago\t{}\t{}\t{}\t{}",
            klatch::domain::secs_between(now, r.ts),
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
        AdminCmd::Librarian { member, off } => {
            t.admin_set_librarian(&member, !off)?;
            println!(
                "{member} is {} a librarian",
                if off { "no longer" } else { "now" }
            );
        }
        AdminCmd::Reviewer { member, off } => {
            t.admin_set_reviewer(&member, !off)?;
            println!(
                "{member} is {} a reviewer",
                if off { "no longer" } else { "now" }
            );
        }
        AdminCmd::AdminAgent { member, off } => {
            t.admin_set_admin_agent(&member, !off)?;
            println!(
                "{member} is {} the admin agent",
                if off { "no longer" } else { "now" }
            );
        }
        AdminCmd::RoomCreation { policy } => {
            let granted = matches!(policy, RoomPolicy::Granted);
            t.admin_set_room_creation_granted(granted)?;
            println!(
                "creating rooms and inviting is now {}",
                if granted {
                    "for members with a grant"
                } else {
                    "open to every member"
                }
            );
        }
        AdminCmd::GrantRooms { member, off } => {
            t.admin_grant_room_capability(&member, !off)?;
            println!(
                "{member} {} create rooms and invite",
                if off { "may no longer" } else { "may now" }
            );
        }
        AdminCmd::RateLimit { posts, secs, off } => {
            let limit = match (off, posts, secs) {
                (true, None, None) => None,
                (false, Some(p), Some(s)) => Some((p, s)),
                _ => bail!("give POSTS and SECS (rate-limit 30 60), or --off alone"),
            };
            t.admin_set_rate_limit(limit)?;
            match limit {
                Some((p, s)) => {
                    println!("each member may post {p} times in {s} seconds in all rooms")
                }
                None => println!("no rate limit"),
            }
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
        AdminCmd::ReviewQueue { limit } => review_queue(t, limit)?,
        AdminCmd::Recovery
        | AdminCmd::Recover { .. }
        | AdminCmd::RotateKey
        | AdminCmd::Backup { .. }
        | AdminCmd::Restore { .. }
        | AdminCmd::Retention { .. }
        | AdminCmd::Archive { .. }
        | AdminCmd::RestoreArchive { .. }
        | AdminCmd::Archives
        | AdminCmd::Export { .. }
        | AdminCmd::SealLabel { .. }
        | AdminCmd::RunArchive { .. } => {
            unreachable!("handled in run, which needs the keystore and not only the gateway")
        }
    }
    Ok(())
}

/// `admin archives` (#487): one tab-separated line per file, then a total. Counts and ages only, never text. A file
/// that does not open, or whose pages fail the check, is shown as UNUSABLE with why and makes the exit status an error,
/// so a cron job notices; the listing still covers every other file first.
fn list_archive_files(dir: &std::path::Path, key: &klatch::keys::Key) -> Result<()> {
    use klatch::store::{ArchiveState, list_archives};
    let found = list_archives(dir, key)?;
    if found.is_empty() {
        println!("no archives in {}", dir.display());
        return Ok(());
    }
    let at = now();
    let days = |t: i64| format!("{}d ago", (at - t).max(0) / 86_400);
    let (mut files, mut messages, mut bytes, mut unusable) = (0, 0, 0, 0);
    for a in &found {
        let size = format!("{} bytes", a.bytes);
        match &a.state {
            ArchiveState::Ok {
                rooms,
                messages: n,
                oldest,
                newest,
            } => {
                files += 1;
                messages += n;
                bytes += a.bytes;
                println!(
                    "{}\tok\t{}\t{n} messages\toldest {}, newest {}\t{size}",
                    a.path.display(),
                    rooms.join(","),
                    days(*oldest),
                    days(*newest)
                );
            }
            ArchiveState::Unreadable(why) => {
                unusable += 1;
                println!("{}\tUNUSABLE\t{why}\t{size}", a.path.display());
            }
            ArchiveState::Partial => println!(
                "{}\tpartial\tleft by an interrupted archive; safe to remove\t{size}",
                a.path.display()
            ),
        }
    }
    println!(
        "{files} archive{}, {messages} messages, {bytes} bytes, {unusable} unusable; nothing is pruned automatically",
        if files == 1 { "" } else { "s" }
    );
    if unusable > 0 {
        bail!("{unusable} archive file(s) cannot be used; restore them from a backup");
    }
    Ok(())
}

/// The commands about the key: they need the keystore as well as the gateway, and each one is in the audit.
fn key_command(db: &std::path::Path, cmd: AdminCmd) -> Result<()> {
    let who = Principal::unattributed("admin");
    let note = |action: &str, outcome: &str| -> Result<()> {
        let gateway = Gateway::new(custody::open_store(db)?, CliClock, AllowAll);
        Ok(gateway.admin_note(&who, (action, ""), outcome)?)
    };
    match cmd {
        AdminCmd::Recovery => {
            let key = custody::current_key(db)?.ok_or_else(|| {
                anyhow::anyhow!("this store has no key yet; run any klatch command to make one")
            })?;
            note("recovery-show", "")?;
            println!("{}", klatch::keys::recovery_code(&key));
            let gateway = Gateway::new(custody::open_store(db)?, CliClock, AllowAll);
            for label in gateway.sealed_labels()? {
                if let Some(k) = custody::label_key(db, &label)? {
                    println!("label {label}: {}", klatch::keys::recovery_code(&k));
                }
            }
            println!(
                "Keep these codes somewhere safe and private. Each restores one key; they are not stored anywhere else."
            );
        }
        AdminCmd::Recover { code, label } => {
            let key = klatch::keys::restore(&code.join(" ")).map_err(|e| anyhow::anyhow!("{e}"))?;
            match &label {
                Some(l) => custody::restore_label_key(db, l, &key)?,
                None => custody::restore_key(db, &key)?,
            }
            note("recover-key", label.as_deref().unwrap_or(""))?;
            println!("the key is back in place");
        }
        AdminCmd::RunArchive { file, run } => {
            let key = custody::current_key(db)?
                .ok_or_else(|| anyhow::anyhow!("this store has no key to open an archive with"))?;
            let text = run::read_archive(&file, &key, &run)?;
            // audited before anything is printed
            note("run-archive", &format!("{run} from {}", file.display()))?;
            print!("{text}");
        }
        AdminCmd::SealLabel { label } => {
            let gateway = Gateway::new(custody::open_store(db)?, CliClock, AllowAll);
            gateway.admin_seal_label(&who, &label)?;
            let made = custody::ensure_label_key(db, &label)?;
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
            let key = custody::current_key(db)?
                .ok_or_else(|| anyhow::anyhow!("this store has no key yet; nothing to back up"))?;
            custody::open_store(db)?.backup_to(&file, &key)?;
            note("backup", &file.to_string_lossy())?;
            println!(
                "wrote an encrypted backup to {} (it opens only with this store's key: keep the recovery code too)",
                file.display()
            );
        }
        AdminCmd::Restore { file } => {
            let key = custody::current_key(db)?.ok_or_else(|| {
                anyhow::anyhow!(
                    "this store has no key; restore the key first with `klatch admin recover`"
                )
            })?;
            klatch::store::encrypt::restore_backup(db, &file, &key)?;
            note("restore", &file.to_string_lossy())?;
            println!(
                "restored {}; the store it replaced is kept as rooms.db.before-restore beside it",
                file.display()
            );
        }
        AdminCmd::Retention { room, days } => {
            let store = custody::open_store(db)?;
            let key = format!("retention:{room}");
            if days == "off" {
                store.policy_set(&key, None, now())?;
                note("retention", &format!("{room} off"))?;
                println!("retention for {room} removed");
            } else {
                let n: i64 = days.parse().ok().filter(|n| *n >= 1).ok_or_else(|| {
                    anyhow::anyhow!("days must be a whole number of at least 1, or off")
                })?;
                store.policy_set(&key, Some(&n.to_string()), now())?;
                note("retention", &format!("{room} {n} days"))?;
                println!(
                    "messages of {room} older than {n} days will be archived by `klatch admin archive`"
                );
            }
        }
        AdminCmd::Archive { room, all_rooms } => {
            let store = custody::open_store(db)?;
            let key = custody::current_key(db)?
                .ok_or_else(|| anyhow::anyhow!("this store has no key yet; nothing to archive"))?;
            let rooms = if all_rooms {
                store.message_rooms()?
            } else {
                vec![room.unwrap_or_default()]
            };
            let dir = db.with_file_name("archive");
            let at = now();
            let mut total = 0;
            for r in rooms {
                let days = match store.policy_get(&format!("retention:{r}"))? {
                    Some(d) => Some(d),
                    None => store.policy_get("retention:*")?,
                };
                let Some(days) = days.and_then(|d| d.parse::<i64>().ok()) else {
                    if !all_rooms {
                        bail!(
                            "{r} has no retention; set one first: klatch admin retention {r} <days>"
                        );
                    }
                    continue;
                };
                std::fs::create_dir_all(&dir)?;
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
                }
                let safe: String = r
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                let file = dir.join(format!("{safe}-{at}.db"));
                let n = store.archive_before(&r, at - days * 86_400, &file, &key)?;
                if n > 0 {
                    println!("{r}: archived {n} messages to {}", file.display());
                    total += n;
                }
            }
            note("archive", &format!("{total} messages"))?;
            if total == 0 {
                println!("nothing is older than its retention");
            }
        }
        AdminCmd::RestoreArchive { file } => {
            let store = custody::open_store(db)?;
            let key = custody::current_key(db)?
                .ok_or_else(|| anyhow::anyhow!("this store has no key; nothing to restore into"))?;
            let n = store.restore_archive(&file, &key)?;
            note("restore-archive", &file.to_string_lossy())?;
            println!("restored {n} messages from {}", file.display());
        }
        AdminCmd::Archives => {
            let dir = db.with_file_name("archive");
            match custody::current_key(db)? {
                Some(key) => list_archive_files(&dir, &key)?,
                None if !dir.exists() => println!("no archives in {}", dir.display()),
                None => bail!(
                    "{} exists but the store key is missing, so no file can be checked; see `klatch admin recover`",
                    dir.display()
                ),
            }
        }
        AdminCmd::Export { room, json } => {
            let gateway = Gateway::new(custody::open_store(db)?, CliClock, AllowAll);
            gateway.use_label_keys(custody::label_ring(db, &gateway.sealed_labels()?)?);
            // audited before anything is printed: the act is recorded even if the output is cut short
            gateway.admin_note(&who, ("export", &room), "readable transcript printed")?;
            for m in gateway.tail(&who, &room, (0, 1_000_000))? {
                if json {
                    println!("{}", export_json_line(&room, &m));
                    continue;
                }
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
            let key = custody::rotate_key(db)?;
            note("rotate-key", "")?;
            println!(
                "the store is now encrypted under a new key. The old key and the old recovery code no longer work."
            );
            println!("New recovery code: {}", klatch::keys::recovery_code(&key));
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
            | AdminCmd::Retention { .. }
            | AdminCmd::Archive { .. }
            | AdminCmd::RestoreArchive { .. }
            | AdminCmd::Archives
            | AdminCmd::Export { .. }
            | AdminCmd::SealLabel { .. }
            | AdminCmd::RunArchive { .. }
    )
}

pub fn run(cmd: AdminCmd) -> Result<()> {
    authorize(&cmd)?;
    let db = db_path()?;
    if needs_the_keystore(&cmd) {
        return key_command(&db, cmd);
    }
    let store = custody::open_store(&db)?;
    let gateway = Rc::new(Gateway::new(store, CliClock, AllowAll));
    // the person administering holds every label key
    gateway.use_label_keys(custody::label_ring(&db, &gateway.sealed_labels()?)?);
    execute(&Direct::new(gateway, Principal::unattributed("admin")), cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_json_export_line_is_one_line_with_every_field() {
        let m = klatch::domain::Message {
            id: "01ID".into(),
            seq: 7,
            sender: "alice".into(),
            attribution: klatch::domain::Attribution::Verified,
            label: "public".into(),
            body: "two\nlines \"quoted\"".into(),
            ts: 99,
            kind: Some("note".into()),
            payload: Some("p".into()),
        };
        let line = export_json_line("lobby", &m);
        assert!(!line.contains('\n'));
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["room"], "lobby");
        assert_eq!(v["seq"], 7);
        assert_eq!(v["body"], "two\nlines \"quoted\"");
        assert_eq!(v["attribution"], "verified");
        assert_eq!(v["kind"], "note");
        assert_eq!(v["payload"], "p");
        assert_eq!(v["id"], "01ID");
    }

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
