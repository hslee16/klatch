//! The store checks: where the store is and whether its key can be read, the archive files beside it, and the
//! permission modes the hooks recorded for its members.

use super::{Report, Status};
use klatch::keys::Key;
use klatch::sync::within;
use std::path::{Path, PathBuf};

/// How long the keystore may take to answer before it is reported as stuck.
const KEYSTORE_PATIENCE: std::time::Duration = std::time::Duration::from_secs(5);

/// The store path as every other command computes it, but without creating the directory.
fn store_path(home: Option<&Path>) -> Option<PathBuf> {
    match std::env::var_os("KLATCH_HOME") {
        Some(p) => Some(PathBuf::from(p).join("rooms.db")),
        None => home.map(|h| h.join(".klatch").join("rooms.db")),
    }
}

/// The store key, asked for on a thread with a time limit; `None` when it is missing, unreadable or slow.
fn key_within_patience(db: &Path) -> Option<Key> {
    let asked = db.to_path_buf();
    within(KEYSTORE_PATIENCE, move || {
        klatch::member::custody::current_key(&asked).ok().flatten()
    })
    .flatten()
}

/// What the keystore says about the store key: its warning, and whether the key exists or why it could not be read.
type KeyProbe = (Option<String>, Result<bool, String>);

/// Asked on a thread with a time limit, and only whether a key exists is brought back: never the key itself.
fn probe_key(db: &Path) -> Option<KeyProbe> {
    let asked = db.to_path_buf();
    within(KEYSTORE_PATIENCE, move || {
        let key = klatch::member::custody::key_home(&asked);
        let found = key
            .source
            .get(&key.name)
            .map(|k| k.is_some())
            .map_err(|e| e.to_string());
        (key.warning.clone(), found)
    })
}

/// Where the key lives, as far as the environment and the platform say.
fn key_place(warned: bool) -> String {
    if let Some(dir) = std::env::var_os("KLATCH_KEY_DIR") {
        format!("key files in {}", PathBuf::from(dir).display())
    } else if std::env::var_os("KLATCH_IDENTITY_FILE").is_some() {
        "this worker's key bundle".to_string()
    } else if cfg!(target_os = "macos") {
        "the macOS Keychain".to_string()
    } else if warned {
        "a key file".to_string()
    } else {
        "the Secret Service".to_string()
    }
}

/// A store that needs no key check: there is none yet, or it is an old plaintext one. `None` when it is encrypted.
fn unkeyed_store(db: &Path) -> Option<(Status, String, Option<&'static str>)> {
    if !db.exists() {
        return Some((
            Status::Ok,
            format!(
                "store: no store yet at {}; it is created on first use",
                db.display()
            ),
            None,
        ));
    }
    klatch::store::encrypt::is_plaintext(db).then(|| {
        (
            Status::Warn,
            format!("store: {} is not encrypted (an old prototype store); klatch converts it, keeping a verified backup, the next time any command opens it", db.display()),
            Some("run any klatch command once, for example: klatch who lobby"),
        )
    })
}

pub(super) fn check_store(r: &mut Report, home: Option<&Path>) {
    let Some(db) = store_path(home) else {
        r.add(
            Status::Fail,
            "store: HOME is not set, so the store cannot be found",
            Some("set HOME or KLATCH_HOME"),
        );
        return;
    };
    if let Some((status, line, fix)) = unkeyed_store(&db) {
        r.add(status, line, fix);
        return;
    }
    let Some((warning, found)) = probe_key(&db) else {
        r.add(
            Status::Fail,
            format!("store: {} is encrypted, but the keystore did not answer within {} seconds; a dialog asking whether klatch may read the key is probably waiting for you", db.display(), KEYSTORE_PATIENCE.as_secs()),
            Some("answer the dialog (choose Always Allow), or run any klatch command once from a terminal; see #371 and #372"),
        );
        return;
    };
    if let Some(w) = &warning {
        r.add(Status::Warn, format!("keystore: {w}"), None);
    }
    report_key(r, &db, &key_place(warning.is_some()), found);
}

/// The store line for an encrypted store: its key is in `place`, is not there, or could not be read.
fn report_key(r: &mut Report, db: &Path, place: &str, found: Result<bool, String>) {
    match found {
        Ok(true) => r.add(Status::Ok, format!("store: {}, encrypted; the key is in {place}", db.display()), None),
        Ok(false) => r.add(
            Status::Fail,
            format!("store: {} is encrypted and its key is not in {place}", db.display()),
            Some("klatch admin recover (it asks for the recovery code); without the key or the code the data cannot be read"),
        ),
        Err(e) => r.add(Status::Fail, format!("store: the key in {place} could not be read: {e}"), Some("check the keystore, or klatch admin recover")),
    }
}

/// The archive files `klatch admin archive` made (#487): each must open with the store key and pass SQLCipher's page
/// check, because a message moved into one is only a tombstone in the live store. Says nothing when there are none.
pub(super) fn check_archives(r: &mut Report, home: Option<&Path>) {
    let Some(db) = store_path(home) else { return };
    let dir = db.with_file_name("archive");
    if !dir.is_dir() {
        return;
    }
    let Some(key) = key_within_patience(&db) else {
        r.add(
            Status::Warn,
            format!("archives: {} exists but the store key could not be read, so the files were not checked", dir.display()),
            Some("fix the store line above first; archives open only with the store key"),
        );
        return;
    };
    let Ok(found) = klatch::store::list_archives(&dir, &key) else {
        r.add(
            Status::Warn,
            format!("archives: {} could not be listed", dir.display()),
            None,
        );
        return;
    };
    report_archives(r, &dir, &found);
}

/// One line for each archive file that is not good, then one line counting the good ones.
fn report_archives(r: &mut Report, dir: &Path, found: &[klatch::store::ArchiveInfo]) {
    use klatch::store::ArchiveState;
    for a in found {
        report_bad_archive(r, a);
    }
    let good = found.iter().filter_map(|a| match &a.state {
        ArchiveState::Ok { messages, .. } => Some((messages, a.bytes)),
        _ => None,
    });
    let (mut files, mut messages, mut bytes) = (0, 0, 0);
    for (n, size) in good {
        files += 1;
        messages += n;
        bytes += size;
    }
    if files > 0 {
        r.add(
            Status::Ok,
            format!("archives: {files} file{}, {messages} messages, {bytes} bytes in {}; each opens with the store key and passes the page check", if files == 1 { "" } else { "s" }, dir.display()),
            None,
        );
    }
}

/// The line for an archive file that is unusable or was left half made; a good one says nothing here.
fn report_bad_archive(r: &mut Report, a: &klatch::store::ArchiveInfo) {
    use klatch::store::ArchiveState;
    match &a.state {
        ArchiveState::Ok { .. } => {}
        ArchiveState::Unreadable(why) => r.add(
            Status::Fail,
            format!("archives: {} is unusable: {why}", a.path.display()),
            Some("restore this file from a backup; messages archived into it stay tombstones in the live store until a good copy can be restored, and nothing else is affected"),
        ),
        ArchiveState::Partial => r.add(
            Status::Warn,
            format!("archives: {} was left by an interrupted `klatch admin archive`", a.path.display()),
            Some("remove it; the live messages were not changed (an archive is put in place before any message becomes a tombstone)"),
        ),
    }
}

/// What a socket nudge does to a member in permission mode `mode` (RESEARCH-NOTES 47.2, #553). A raw socket sender
/// cannot attest a mode, so Claude Code holds its message for a session that bypasses prompts and accepts it for one
/// that prompts. Only modes that were measured are called prompting; anything else is unknown, never fine.
/// How many rows the tables that only grow hold (#487), so a store that is getting large is seen before it is slow.
/// Information only, never a warning: how big is too big, and what to prune, is retention policy (#568, the human's).
/// Read-only, and says nothing when there is no readable store.
pub(super) fn check_growth(r: &mut Report, home: Option<&Path>) {
    let Some(db) = store_path(home).filter(|d| d.exists()) else {
        return;
    };
    if klatch::store::encrypt::is_plaintext(&db) {
        return;
    }
    let asked = db.clone();
    let counts = within(KEYSTORE_PATIENCE, move || {
        let key = klatch::member::custody::current_key(&asked)
            .ok()
            .flatten()?;
        klatch::store::SqliteStore::read_growth(&asked, &key).ok()
    })
    .flatten();
    let Some(counts) = counts else { return };
    let parts: Vec<String> = counts.iter().map(|(l, n)| format!("{n} {l}")).collect();
    r.add(
        Status::Ok,
        format!(
            "growth: {} (live rows; nothing here is pruned automatically)",
            parts.join(", ")
        ),
        None,
    );
}

/// The failed proofs of the last 24 hours across all terminals (#1310): a warning while there are as many as the
/// alarm level, counts only. The same count the alarm row of the audit is written at; it locks nothing. Says
/// nothing below the level or when there is no readable store.
pub(super) fn check_proof_failures(r: &mut Report, home: Option<&Path>) {
    let Some(db) = store_path(home).filter(|d| d.exists()) else {
        return;
    };
    if klatch::store::encrypt::is_plaintext(&db) {
        return;
    }
    let now = klatch::member::now();
    let asked = db.clone();
    let census = within(KEYSTORE_PATIENCE, move || {
        let key = klatch::member::custody::current_key(&asked)
            .ok()
            .flatten()?;
        klatch::proof::alarm::read_recent(&asked, &key, now).ok()
    })
    .flatten();
    report_proof_failures(r, census);
}

/// The warning for `census` (failures, distinct terminals): nothing when the store could not be read, or below the level.
pub(super) fn report_proof_failures(r: &mut Report, census: Option<(u64, u64)>) {
    let Some((failures, terminals)) = census else {
        return;
    };
    if failures >= klatch::proof::alarm::CEILING {
        r.add(
            Status::Warn,
            format!("proof failures: {failures} wrong admin passphrases or recovery codes in the last 24 hours, from {terminals} terminals (the alarm level is {}); nothing is locked by this, the wait of each terminal is unchanged", klatch::proof::alarm::CEILING),
            Some("if these were not you, someone is guessing: see the failed proofs in `klatch admin audit`, find what is opening terminals, and consider a longer passphrase (`klatch admin init-passphrase`)"),
        );
    }
}

/// Whether the admin passphrase is set (#1262): a warning while it is not, because until then the first
/// `klatch admin init-passphrase` proves nothing and a program with a terminal could run it before the human. Reads
/// the store read-only; sets, issues and asks for nothing. Says nothing when there is no readable store.
pub(super) fn check_passphrase(r: &mut Report, home: Option<&Path>) {
    let Some(db) = store_path(home).filter(|d| d.exists()) else {
        return;
    };
    if klatch::store::encrypt::is_plaintext(&db) {
        return;
    }
    let asked = db.clone();
    let set = within(KEYSTORE_PATIENCE, move || {
        let key = klatch::member::custody::current_key(&asked)
            .ok()
            .flatten()?;
        klatch::store::SqliteStore::read_policy_has(&asked, &key, klatch::proof::VERIFIER_KEY).ok()
    })
    .flatten();
    report_passphrase(r, set);
}

/// The warning for `set` (`None` when it could not be read): only an unset passphrase is reported.
pub(super) fn report_passphrase(r: &mut Report, set: Option<bool>) {
    if set == Some(false) {
        r.add(
            Status::Warn,
            "admin passphrase: no admin passphrase is set; until you set one, any program that can open a terminal as your user could set it first",
            Some("run `klatch admin init-passphrase` yourself, in a terminal; doctor and init never set it"),
        );
    }
}

fn nudge_effect(mode: &str) -> Status {
    match klatch::domain::wake_path(Some(mode)) {
        klatch::domain::WakePath::Nudge => Status::Ok,
        klatch::domain::WakePath::HookAndWatcher => Status::Warn,
    }
}

/// The permission modes the hooks recorded: member, mode, and when. `None` when the store or its key cannot be read,
/// which the store line already reports.
fn recorded_modes(db: &Path) -> Option<Vec<(String, String, i64)>> {
    let asked = db.to_path_buf();
    within(KEYSTORE_PATIENCE, move || {
        let key = klatch::member::custody::current_key(&asked)
            .ok()
            .flatten()?;
        klatch::store::SqliteStore::read_member_modes(&asked, &key).ok()
    })
    .flatten()
}

/// Members whose last hook input said they bypass permission prompts: a socket nudge to them is held (#553). Reads
/// the modes the hooks recorded, read-only, and writes nothing, settings included (the choice is #552, the human's).
pub(super) fn check_modes(r: &mut Report, home: Option<&Path>) {
    let Some(db) = store_path(home).filter(|d| d.exists()) else {
        return;
    };
    if klatch::store::encrypt::is_plaintext(&db) {
        return;
    }
    let Some(modes) = recorded_modes(&db) else {
        return;
    };
    for (member, mode, _) in &modes {
        report_mode(r, member, mode);
    }
}

fn report_mode(r: &mut Report, member: &str, mode: &str) {
    match nudge_effect(mode) {
        Status::Ok => r.add(
            Status::Ok,
            format!("member {member}, permission mode {mode}: a socket nudge reaches it"),
            None,
        ),
        _ if mode == "bypassPermissions" => r.add(
            Status::Warn,
            format!("member {member}, permission mode {mode}: a socket nudge to it is held by Claude Code (no-mode-asserted) and dropped after about 5 minutes unless a human answers its dialog; the hooks still deliver, so mail is not lost"),
            Some("nothing is broken. Setting crossSessionInbound to \"accept\" in the user settings would let nudges through, but it widens what any process of yours can put before a model that bypasses prompts, so only you can decide that (#552); klatch never writes it"),
        ),
        _ => r.add(
            Status::Warn,
            format!("member {member}, permission mode {mode} is unknown to klatch: it cannot say whether a socket nudge to this member is held"),
            Some("the hooks still deliver; report the mode on the klatch tracker so it can be classed"),
        ),
    }
}

/// Recovery attempts recorded beside the store and not settled (#1636): failed guesses at the recovery code, which
/// are audited only when a recovery later succeeds and the store opens, are otherwise visible only in that file.
/// A warning while there is any, with the count. Reads the file only (no store, no key), so it also works when the
/// store cannot be read, which is when the file is written.
pub(super) fn check_recovery_attempts(r: &mut Report, home: Option<&Path>) {
    if let Some(db) = store_path(home) {
        check_attempts_file(r, &db);
    }
}

pub(super) fn check_attempts_file(r: &mut Report, db: &Path) {
    let n = klatch::proof::recovery_attempts_recorded(db);
    report_recovery_attempts(r, n, &klatch::proof::recovery_attempts_file(db));
}

pub(super) fn report_recovery_attempts(r: &mut Report, n: u32, file: &Path) {
    if n == 0 {
        return;
    }
    r.add(
        Status::Warn,
        format!(
            "{n} recovery attempts are recorded in {} and no recovery has settled them yet (a count of {} means the file could not be read safely: a link, a pipe or one over the size cap)",
            file.display(),
            u32::MAX
        ),
        Some("if these were not you, someone is guessing the recovery code (it carries 256 bits, so this is detection, not a risk of success); the file is audited and removed the next time a recovery succeeds and the store opens"),
    );
}
