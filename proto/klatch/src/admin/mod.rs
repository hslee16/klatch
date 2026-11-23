//! `klatch admin ...`: the controls a person uses to stop things (R11, ticket #23): close or reopen a room, pause or
//! resume every room, remove a member from a room (or from all rooms) and allow it back, and read the audit.
//!
//! Who may administer is proved, not guessed (#1233): every command first asks for the admin passphrase on a real
//! terminal (`prove::require`, `klatch::proof`), and only the `AdminProof` it returns reaches the gateway, as the
//! principal `human` (`Verified`) that `RequireHuman` accepts. The destructive commands then also ask for the target
//! typed back. The terminal-and-process-tree heuristic this replaced is gone.

use super::*;
use clap::{Subcommand, ValueEnum};
use route::{Passphrase, Route};
use std::io::{BufRead, Write};

mod acts;
mod archive;
mod export;
mod keys;
mod listing;
pub(crate) mod prove;
mod route;
mod shell;
mod stats;

#[cfg(test)]
mod command_tests;

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
    /// Name a member the grantor of a room (it may grant room-scoped clearance there, `klatch grant`), replacing the
    /// room's creator or any other; with --off take the role from that member
    Grantor {
        room: String,
        member: String,
        #[arg(long)]
        off: bool,
    },
    /// Map an ssh forced-command key label to one member: `klatch serve --stdio --ssh-key LABEL`, the command in an
    /// `authorized_keys` line, then acts as that member and no other (a floor act: never an agent's); with --off take
    /// the mapping from that member
    SshKey {
        label: String,
        member: String,
        #[arg(long)]
        off: bool,
    },
    /// Set a member's clearance in one room only, at most the agent cap; with --off revoke it
    RoomClear {
        room: String,
        member: String,
        label: Option<String>,
        #[arg(long, conflicts_with = "label", required_unless_present = "label")]
        off: bool,
    },
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
    /// Make a member the facilitator (routes asks into topical rooms: it may read, create a room, invite and post the
    /// invite pointer, and nothing else; it never grants or labels), or with --off an ordinary member
    Facilitator {
        member: String,
        #[arg(long)]
        off: bool,
    },
    /// Make a member an integrator (the one role that may take the `merge:main` claim that serializes merging; it is
    /// otherwise an ordinary member, and the klatch binary does no code-host work for it), or with --off an ordinary
    /// member
    Integrator {
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
    /// `rate-limit --off` lifts it. Off by default. With `--member NAME` the limit is that member's own, which wins
    /// over the global one (higher or lower); `rate-limit --member NAME --off` makes the member inherit again.
    /// `rate-limit --list` shows the global limit and every member limit in force, one per line
    RateLimit {
        posts: Option<usize>,
        secs: Option<i64>,
        #[arg(long)]
        off: bool,
        #[arg(long, conflicts_with_all = ["posts", "secs", "off", "member"])]
        list: bool,
        #[arg(long)]
        member: Option<String>,
    },
    /// How long a holder that is busy with no sign of progress keeps its claims, in seconds (1 to 604800), for
    /// example `lease-cap 86400`; `lease-cap --off` puts back the default of 4 hours
    LeaseCap {
        secs: Option<i64>,
        #[arg(long)]
        off: bool,
    },
    /// How long the claims of a holder found gone linger, in seconds (1 to 86400), for example `lease-grace 60`;
    /// `lease-grace --off` puts back the default of 5 minutes
    LeaseGrace {
        secs: Option<i64>,
        #[arg(long)]
        off: bool,
    },
    /// How long an ask may sit with no claim in ROOM before its asker is told, in seconds (10 to 3600), for example
    /// `room-stale lobby 120`; `room-stale lobby --off` puts back the default of 60 seconds
    RoomStale {
        room: String,
        secs: Option<i64>,
        #[arg(long)]
        off: bool,
    },
    /// The lease an `ask:<N>` claim gets in ROOM when none is asked for, in seconds (10 to 300, the most an ask claim
    /// ever lasts), for example `room-ask-lease lobby 120`; `room-ask-lease lobby --off` puts back the default of 90
    RoomAskLease {
        room: String,
        secs: Option<i64>,
        #[arg(long)]
        off: bool,
    },
    /// Require harness capabilities of ROOM as MODE:CAPABILITY[,CAPABILITY...] with MODE refuse (a member whose harness
    /// lacks one cannot join) or flag (it joins and the gap is recorded), for example
    /// `room-require lobby refuse:pre-action-enforcement,wake`; `room-require lobby --off` lifts it. Only `native`
    /// support counts; a name this build does not know is refused (#61)
    RoomRequire {
        room: String,
        spec: Option<String>,
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
    /// End the canary of a room (planted by `admin label` to private or above, or by `admin canary`); a later
    /// `admin label` plants a new one
    CanaryRetire { room: String },
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
    /// (its clearance reaches the label and the key was handed to it) can read or write them. The messages already
    /// under the label are sealed too, in the same step, unless `--keep-plain`
    SealLabel {
        label: String,
        /// Leave the messages already under the label as plain text
        #[arg(long)]
        keep_plain: bool,
    },
    /// Re-encrypt the store under a new key; the old key (and the old recovery code) stop working
    RotateKey,
    /// Move a sealed label to a new key: every message of the label, in the store and in the archive files, is sealed
    /// again under it, and the old key (and its recovery code) open none of them. Workers that hold the label must be
    /// relaunched to get the new key; backups and archive copies made earlier keep the old key's text
    RotateLabelKey { label: String },
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
    /// Set the admin passphrase every admin, worker, anchor, run and judge command asks for (typed twice, at a terminal,
    /// never from an argument or the environment; only a verifier is kept). With one set, change it: the old one is
    /// asked for first
    InitPassphrase,
    /// Set a new admin passphrase when the old one is forgotten, proved by the store's recovery code (`admin recovery`)
    ResetPassphrase,
    /// Show or set how long one typed passphrase stays good for the commands of ONE long-lived process, in seconds
    /// (10 to 1800, default 60; `--off` puts the default back). Only `admin shell` is such a process
    UnlockWindow {
        secs: Option<i64>,
        #[arg(long, conflicts_with = "secs")]
        off: bool,
    },
    /// Read admin, worker, anchor, run and judge commands from the terminal under one passphrase, held in this
    /// process's memory for the unlock window and nowhere else; `exit` or end of input leaves it
    Shell,
    /// Print a counts-only usage report: messages per room and day, senders and attribution, kinds, asks (created,
    /// answered, closed, time to first answer), claims by class, refusals by rule, direct message and private rooms,
    /// members, hook runs, admin acts and worker credentials. Never message text, ask text, claim detail, labels or
    /// payloads; it does name rooms and members, so it asks for the passphrase and needs your terminal. `--since` is a
    /// day as YYYY-MM-DD (UTC); `--room` narrows the room-scoped counts; `--json` prints one JSON object for the ledger
    Stats {
        #[arg(long)]
        room: Option<String>,
        #[arg(long)]
        since: Option<String>,
        /// How many senders to list, most messages first
        #[arg(long, default_value_t = 10)]
        top: usize,
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

/// What must be typed back to confirm, if the command is destructive.
fn confirmation(cmd: &AdminCmd) -> Option<String> {
    match cmd {
        AdminCmd::Pause => Some("pause".into()),
        AdminCmd::Close { room } => Some(room.clone()),
        AdminCmd::Recovery => Some("recovery".into()),
        AdminCmd::Recover { .. } => Some("recover".into()),
        AdminCmd::RotateKey => Some("rotate".into()),
        AdminCmd::RotateLabelKey { label } => Some(label.clone()),
        AdminCmd::Restore { .. } => Some("restore".into()),
        AdminCmd::Export { room, .. } => Some(room.clone()),
        AdminCmd::RunArchive { run, .. } => Some(run.clone()),
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

/// Ask for the target typed back when the act is destructive. The line comes from the TERMINAL, never from standard
/// input when that is a pipe: a program sharing the person's terminal cannot confirm for them.
fn confirm(expected: Option<String>, what: &str) -> Result<()> {
    use klatch::proof::Prompt;
    let Some(expected) = expected else {
        return Ok(());
    };
    let line = klatch::proof::TtyPrompt
        .line(&confirm_label(&expected, what))
        .map_err(anyhow::Error::from)?;
    if !typed_back(&expected, &line) {
        bail!("not confirmed; nothing was changed");
    }
    Ok(())
}

/// The label of the typed-back confirmation: what is typed and what it confirms, in text from arguments cleaned the
/// same way as the passphrase prompt. A name that had to be cleaned (so two lookalikes could read the same) also
/// shows its length and a digest of the real name.
fn confirm_label(expected: &str, what: &str) -> String {
    use sha2::{Digest, Sha256};
    let shown = prove::plain(expected);
    if shown == expected {
        format!("type {shown} to confirm `{what}`")
    } else {
        let digest = data_encoding::HEXLOWER.encode(&Sha256::digest(expected.as_bytes()));
        format!(
            "type {shown} ({} chars, sha256 {}) to confirm `{what}`",
            expected.chars().count(),
            &digest[..8]
        )
    }
}

/// What the passphrase approves when the command is the admin shell.
const SHELL_APPROVAL: &str =
    "open an admin shell: an unlock window for ALL admin and launcher commands";

/// The acts that print a secret or stored text: a recovery code, the plaintext of a room or an archive, the audit,
/// the refusals, the review queue, the scrub terms, the archive listing, the rate-limit listing. Run with standard output going anywhere
/// but THIS process's controlling terminal (a pipe, a file, or a pty someone else holds the other end of), the
/// person would approve a prompt for something another program reads, so they are refused before anyone is asked
/// (#1233).
fn prints_stored_text(route: &Route) -> bool {
    use route::{Act, ArchiveAct, KeyAct, Keystore, ListingAct, PolicyAct, ScrubAct};
    matches!(
        route,
        Route::Gateway(Act::Listing(
            ListingAct::Refusals { .. } | ListingAct::Audit { .. } | ListingAct::ReviewQueue { .. }
        )) | Route::Gateway(Act::Policy(PolicyAct::RateLimit { list: true, .. }))
            | Route::Gateway(Act::Scrub(ScrubAct::List))
            | Route::Keystore(Keystore::Archive(ArchiveAct::Archives))
            | Route::Keystore(Keystore::Key(
                KeyAct::Recovery
                    | KeyAct::Export { .. }
                    | KeyAct::RunArchive { .. }
                    | KeyAct::Stats { .. }
                    | KeyAct::RotateKey
                    | KeyAct::RotateLabelKey { .. }
            ))
    )
}

pub fn run(cmd: AdminCmd) -> Result<()> {
    let typed_target = confirmation(&cmd);
    // What the passphrase approves. A recovery code is hidden by what the command PARSED as, never by looking at the
    // words: whatever flags or `--` sit before it, and a room or member named `recover` under another command shows.
    let what = if matches!(cmd, AdminCmd::Recover { .. }) {
        "admin recover <code hidden>".to_string()
    } else {
        prove::describe()
    };
    let db = db_path()?;
    let route = Route::from(cmd);
    if prints_stored_text(&route) && !klatch::proof::output_is_the_prompt_terminal() {
        bail!(
            "this command prints stored text or a secret, and standard output (and standard error) must be your own terminal: a pipe, a file, another pty, or a redirected standard error is refused, so nothing was asked or done; run it yourself in a terminal with standard output and standard error not redirected"
        );
    }
    match route {
        // the two acts that cannot ask for the passphrase: there is none yet, or it is forgotten
        Route::Passphrase(Passphrase::Init) => prove::init_passphrase(&db),
        Route::Passphrase(Passphrase::Reset) => prove::reset_passphrase(&db),
        // the shell reads its commands from standard input, so that must itself be a terminal: a pipe, even with a
        // terminal shared with a person, is refused before anyone is asked for anything
        Route::Passphrase(Passphrase::Shell) => {
            shell::need_a_terminal()?;
            let (_store, _proof) = prove::open_and_require_for(&db, SHELL_APPROVAL)?;
            shell::run()
        }
        // `recover` (the store key) and `restore` must work when the store cannot be read, which is when the
        // verifier cannot be either: the recovery code and the typed confirmation approve them then
        Route::Keystore(k) if needs_store_less_proof(&k) && !prove::store_readable(&db) => {
            run_store_less(&db, k, typed_target, &what)
        }
        route => run_proved(&db, route, typed_target, &what),
    }
}

/// `recover` or `restore` of a store that cannot be read: the recovery code and the typed confirmation approve it.
fn run_store_less(
    db: &std::path::Path,
    k: route::Keystore,
    typed_target: Option<String>,
    what: &str,
) -> Result<()> {
    let proof = prove::recovery_proof(db, &k)?;
    confirm(typed_target, what)?;
    let done = keys::key_command(db, &proof, k);
    if done.is_ok()
        && let Ok(store) = custody::open_store(db)
    {
        klatch::proof::settle_recovery_attempts(db, &store);
    }
    done
}

/// Every other act: the passphrase proof, then the typed confirmation, then the act.
fn run_proved(
    db: &std::path::Path,
    route: Route,
    typed_target: Option<String>,
    what: &str,
) -> Result<()> {
    let (store, proof) = prove::open_and_require_for(db, what)?;
    confirm(typed_target, what)?;
    match route {
        Route::Passphrase(Passphrase::Window { secs, off }) => {
            prove::unlock_window(&store, &proof, secs, off)
        }
        Route::Passphrase(_) => unreachable!("handled above"),
        Route::Keystore(k) => keys::key_command(db, &proof, k),
        Route::Gateway(act) => {
            let gateway = Rc::new(Gateway::new(store, CliClock, RequireHuman::new(AllowAll)));
            // the person administering holds every label key
            gateway.use_label_keys(custody::label_ring(db, &gateway.sealed_labels()?)?);
            acts::execute(&Direct::new(gateway, proof.principal()), act)
        }
    }
}

/// The keystore acts that run exactly when the store may be unreadable: the store key's recovery and a restore.
fn needs_store_less_proof(k: &route::Keystore) -> bool {
    use route::{KeyAct, Keystore};
    matches!(
        k,
        Keystore::Key(KeyAct::Recover { label: None, .. } | KeyAct::Restore { .. })
    )
}

#[cfg(test)]
mod tests {
    use super::export::export_json_line;
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
            advisory: false,
            scope: None,
            refs: Vec::new(),
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

#[cfg(test)]
mod confirm_label_tests {
    use super::confirm_label;

    #[test]
    fn a_plain_name_is_shown_as_it_is_and_a_lookalike_gets_its_length_and_digest() {
        let plain = confirm_label("lobby", "admin close lobby");
        assert_eq!(plain, "type lobby to confirm `admin close lobby`");
        let a = confirm_label("caf\u{e9}", "admin close caf?");
        let b = confirm_label("cafe\u{301}", "admin close caf?");
        assert!(a.contains("caf?") && a.contains("sha256"), "{a}");
        assert_ne!(a, b, "two lookalike names must not read the same");
    }
}
