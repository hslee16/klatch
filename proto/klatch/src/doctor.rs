//! `klatch doctor` (#66): a read-only health check. It looks at this binary, the store and its key, the hooks in the
//! Claude Code settings, the session it runs in and the harness registry, and says for each what works, what does not
//! and why, with the fix. It creates nothing (no store, no key, no settings) and never prints a key.

use crate::init::{HookKind, hook_program, installed_hooks};
use klatch::harness::PsTable;
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    Warn,
    Fail,
}

struct Report {
    lines: Vec<(Status, String, Option<String>)>,
}

impl Report {
    fn add(&mut self, status: Status, line: impl Into<String>, fix: Option<&str>) {
        self.lines
            .push((status, line.into(), fix.map(String::from)));
    }
}

/// The settings file's content, if it exists and parses; `Err` says why not.
fn settings(path: &Path) -> Result<Option<Value>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| format!("not valid JSON ({e})"))
}

/// The store path as every other command computes it, but without creating the directory.
fn store_path(home: Option<&Path>) -> Option<PathBuf> {
    match std::env::var_os("KLATCH_HOME") {
        Some(p) => Some(PathBuf::from(p).join("rooms.db")),
        None => home.map(|h| h.join(".klatch").join("rooms.db")),
    }
}

/// Run `probe` on its own thread and wait up to `limit` for its answer; `None` when it does not come. A keystore can
/// block for ever on a dialog nobody is there to answer, and a health check must not hang with it. The abandoned thread
/// ends with the process.
fn within<T: Send + 'static>(
    limit: std::time::Duration,
    probe: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(probe());
    });
    rx.recv_timeout(limit).ok()
}

/// How long the keystore may take to answer before it is reported as stuck.
const KEYSTORE_PATIENCE: std::time::Duration = std::time::Duration::from_secs(5);

fn check_store(r: &mut Report, home: Option<&Path>) {
    let Some(db) = store_path(home) else {
        r.add(
            Status::Fail,
            "store: HOME is not set, so the store cannot be found",
            Some("set HOME or KLATCH_HOME"),
        );
        return;
    };
    if !db.exists() {
        r.add(
            Status::Ok,
            format!(
                "store: no store yet at {}; it is created on first use",
                db.display()
            ),
            None,
        );
        return;
    }
    if klatch::store::encrypt::is_plaintext(&db) {
        r.add(
            Status::Warn,
            format!("store: {} is not encrypted (an old prototype store); klatch converts it, keeping a verified backup, the next time any command opens it", db.display()),
            Some("run any klatch command once, for example: klatch who lobby"),
        );
        return;
    }
    // asked on a thread with a time limit, and only whether a key exists is brought back: never the key itself
    let asked = db.clone();
    let probe = within(KEYSTORE_PATIENCE, move || {
        let key = crate::custody::key_home(&asked);
        let found = key
            .source
            .get(&key.name)
            .map(|k| k.is_some())
            .map_err(|e| e.to_string());
        (key.warning.clone(), found)
    });
    let Some((warning, found)) = probe else {
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
    let where_ = if let Some(dir) = std::env::var_os("KLATCH_KEY_DIR") {
        format!("key files in {}", PathBuf::from(dir).display())
    } else if std::env::var_os("KLATCH_IDENTITY_FILE").is_some() {
        "this worker's key bundle".to_string()
    } else if cfg!(target_os = "macos") {
        "the macOS Keychain".to_string()
    } else if warning.is_some() {
        "a key file".to_string()
    } else {
        "the Secret Service".to_string()
    };
    match found {
        Ok(true) => r.add(Status::Ok, format!("store: {}, encrypted; the key is in {where_}", db.display()), None),
        Ok(false) => r.add(
            Status::Fail,
            format!("store: {} is encrypted and its key is not in {where_}", db.display()),
            Some("klatch admin recover (it asks for the recovery code); without the key or the code the data cannot be read"),
        ),
        Err(e) => r.add(Status::Fail, format!("store: the key in {where_} could not be read: {e}"), Some("check the keystore, or klatch admin recover")),
    }
}

/// The archive files `klatch admin archive` made (#487): each must open with the store key and pass SQLCipher's page
/// check, because a message moved into one is only a tombstone in the live store. Says nothing when there are none.
fn check_archives(r: &mut Report, home: Option<&Path>) {
    use klatch::store::{ArchiveState, list_archives};
    let Some(db) = store_path(home) else { return };
    let dir = db.with_file_name("archive");
    if !dir.is_dir() {
        return;
    }
    let asked = db.clone();
    let key = within(KEYSTORE_PATIENCE, move || {
        crate::custody::current_key(&asked).ok().flatten()
    })
    .flatten();
    let Some(key) = key else {
        r.add(
            Status::Warn,
            format!("archives: {} exists but the store key could not be read, so the files were not checked", dir.display()),
            Some("fix the store line above first; archives open only with the store key"),
        );
        return;
    };
    let Ok(found) = list_archives(&dir, &key) else {
        r.add(
            Status::Warn,
            format!("archives: {} could not be listed", dir.display()),
            None,
        );
        return;
    };
    let (mut files, mut messages, mut bytes) = (0, 0, 0);
    for a in &found {
        match &a.state {
            ArchiveState::Ok { messages: n, .. } => {
                files += 1;
                messages += n;
                bytes += a.bytes;
            }
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
    if files > 0 {
        r.add(
            Status::Ok,
            format!("archives: {files} file{}, {messages} messages, {bytes} bytes in {}; each opens with the store key and passes the page check", if files == 1 { "" } else { "s" }, dir.display()),
            None,
        );
    }
}

/// What a socket nudge does to a member in permission mode `mode` (RESEARCH-NOTES 47.2, #553). A raw socket sender
/// cannot attest a mode, so Claude Code holds its message for a session that bypasses prompts and accepts it for one
/// that prompts. Only modes that were measured are called prompting; anything else is unknown, never fine.
fn nudge_effect(mode: &str) -> Status {
    match mode {
        "bypassPermissions" => Status::Warn,
        "default" | "acceptEdits" | "plan" | "auto" | "dontAsk" => Status::Ok,
        _ => Status::Warn,
    }
}

/// Members whose last hook input said they bypass permission prompts: a socket nudge to them is held (#553). Reads
/// the modes the hooks recorded, read-only, and writes nothing, settings included (the choice is #552, the human's).
fn check_modes(r: &mut Report, home: Option<&Path>) {
    let Some(db) = store_path(home).filter(|d| d.exists()) else {
        return;
    };
    if klatch::store::encrypt::is_plaintext(&db) {
        return;
    }
    let asked = db.clone();
    let modes = within(KEYSTORE_PATIENCE, move || {
        let key = crate::custody::current_key(&asked).ok().flatten()?;
        klatch::store::SqliteStore::read_member_modes(&asked, &key).ok()
    })
    .flatten();
    // an unreadable store or key is already reported by the store line
    let Some(modes) = modes else { return };
    for (member, mode, _) in &modes {
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
}

fn check_hooks(r: &mut Report, project_dir: &Path, home: Option<&Path>, exe: &Path) {
    let mut places = vec![(
        "project",
        project_dir.join(".claude").join("settings.local.json"),
    )];
    if let Some(h) = home {
        places.push(("user", h.join(".claude").join("settings.json")));
    }
    let (mut any, mut wake, mut enforce) = (false, false, false);
    for (label, path) in places {
        let value = match settings(&path) {
            Ok(Some(v)) => v,
            Ok(None) => continue,
            Err(e) => {
                r.add(
                    Status::Warn,
                    format!("hooks ({label}): {} could not be read: {e}", path.display()),
                    None,
                );
                continue;
            }
        };
        let installed = installed_hooks(&value);
        if installed.is_empty() {
            continue;
        }
        any = true;
        wake |= installed.iter().any(|h| h.2 == HookKind::Wake);
        enforce |= installed.iter().any(|h| h.2 == HookKind::Enforce);
        let mut problem: Option<(Status, String)> = None;
        for (event, cmd, _) in &installed {
            let program = hook_program(cmd);
            if !Path::new(program).is_file() {
                problem = Some((
                    Status::Fail,
                    format!("the {event} hook runs {program}, which does not exist"),
                ));
                break;
            }
            let same = std::fs::canonicalize(program).ok() == std::fs::canonicalize(exe).ok();
            if !same && problem.is_none() {
                problem = Some((
                    Status::Warn,
                    format!(
                        "the {event} hook runs {program}, not this binary ({})",
                        exe.display()
                    ),
                ));
            }
        }
        match problem {
            Some((status, why)) => r.add(
                status,
                format!("hooks ({label}): {why}"),
                Some(&format!(
                    "run klatch init --scope {label} again; it rewrites the path to this binary"
                )),
            ),
            None => {
                let mut events: Vec<&str> = installed.iter().map(|h| h.0.as_str()).collect();
                events.sort_unstable();
                events.dedup();
                r.add(
                    Status::Ok,
                    format!(
                        "hooks ({label}): klatch runs on {} from {}",
                        events.join(", "),
                        path.display()
                    ),
                    None,
                );
            }
        }
    }
    if !any {
        r.add(
            Status::Warn,
            "hooks: klatch's hooks are not installed in the project or user settings (they are only needed for Claude Code sessions)",
            Some("klatch init"),
        );
        return;
    }
    r.add(
        Status::Ok,
        format!(
            "wake hook: {}",
            if wake {
                "on"
            } else {
                "off (klatch init --wake turns it on)"
            }
        ),
        None,
    );
    r.add(
        Status::Ok,
        format!(
            "enforce hook: {}",
            if enforce {
                "on"
            } else {
                "off (klatch init --enforce turns it on)"
            }
        ),
        None,
    );
}

fn check_session(r: &mut Report) {
    match crate::identity::discover_session() {
        Ok(found) => r.add(
            Status::Ok,
            format!(
                "session: this process runs in the session {}",
                found.name.as_deref().unwrap_or(&found.session_id)
            ),
            None,
        ),
        Err(_) => r.add(
            Status::Ok,
            "session: not inside a coding-agent session (nothing to check)",
            None,
        ),
    }
}

fn check_registry(r: &mut Report) {
    let Some((registry, domain)) = crate::identity::registry_and_domain() else {
        return;
    };
    if !registry.dir.exists() {
        r.add(
            Status::Ok,
            format!(
                "registry: no harness registry at {} (Claude Code has not run here)",
                registry.dir.display()
            ),
            None,
        );
        return;
    }
    let found = klatch::harness::list(&registry, &PsTable, domain);
    r.add(
        Status::Ok,
        format!(
            "registry: {} live session(s) (klatch sessions lists them)",
            found.live.len()
        ),
        None,
    );
    check_liveness(r, &found.live, domain);
}

/// Which layers of the liveness probe (DESIGN section 43) can be read on this machine, tried on the live sessions.
fn check_liveness(r: &mut Report, live: &[klatch::harness::Record], domain: &str) {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let host = klatch::harness::SysHost::standard(Path::new(&home));
    let test = klatch::harness::self_test(live, &PsTable, domain, &host);
    if test.sessions == 0 {
        r.add(
            Status::Ok,
            "liveness: no live session to try the probe on",
            None,
        );
        return;
    }
    let works: Vec<String> = test
        .layers
        .iter()
        .map(|l| format!("{} {}/{}", l.layer, l.readable, test.sessions))
        .collect();
    let line = format!("liveness probe layers readable: {}", works.join(", "));
    let broken = test.broken();
    if broken.is_empty() {
        r.add(Status::Ok, line, None);
        return;
    }
    r.add(
        Status::Warn,
        format!(
            "{line}; unreadable for every session: {} (a Claude Code change to the registry or transcript layout?)",
            broken.join(", ")
        ),
        Some(
            "nothing breaks: an unreadable layer counts as unknown, never as a dead session, and leases fall back on their time limits; report the layer on the klatch tracker (#311)",
        ),
    );
}

/// Run every probe, print the report and return whether nothing failed.
pub fn run(
    project_dir: &Path,
    home: Option<&Path>,
    exe: &Path,
    out: &mut dyn Write,
) -> std::io::Result<bool> {
    let mut r = Report { lines: Vec::new() };
    r.add(
        Status::Ok,
        format!(
            "binary: klatch {} at {}",
            crate::version_text(),
            exe.display()
        ),
        None,
    );
    check_store(&mut r, home);
    check_archives(&mut r, home);
    check_modes(&mut r, home);
    check_hooks(&mut r, project_dir, home, exe);
    check_session(&mut r);
    check_registry(&mut r);
    let count = |s: Status| r.lines.iter().filter(|l| l.0 == s).count();
    for (status, line, fix) in &r.lines {
        let tag = match status {
            Status::Ok => "ok  ",
            Status::Warn => "warn",
            Status::Fail => "FAIL",
        };
        writeln!(out, "{tag}  {line}")?;
        if let Some(fix) = fix.as_ref().filter(|_| *status != Status::Ok) {
            writeln!(out, "      fix: {fix}")?;
        }
    }
    let failures = count(Status::Fail);
    writeln!(
        out,
        "\n{} ok, {} warnings, {failures} failures",
        count(Status::Ok),
        count(Status::Warn)
    )?;
    Ok(failures == 0)
}

#[cfg(test)]
mod tests {
    use super::within;
    use std::time::{Duration, Instant};

    #[test]
    fn a_probe_that_answers_in_time_gives_its_answer() {
        assert_eq!(within(Duration::from_secs(5), || 7), Some(7));
    }

    #[test]
    fn a_probe_that_never_answers_is_given_up_on_at_the_limit() {
        let started = Instant::now();
        let got = within(Duration::from_millis(100), || {
            std::thread::sleep(Duration::from_secs(30));
            1
        });
        assert_eq!(got, None);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "waited {:?}",
            started.elapsed()
        );
    }
}
