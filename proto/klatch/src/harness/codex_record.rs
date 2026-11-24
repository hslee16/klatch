//! The Codex registry record (#884): what lets klatch show that a session runs inside a Codex process, found the way
//! the Claude Code registry finds a Claude one, with a record klatch itself writes in its own directory.
//!
//! Same-user evidence, like the Claude registry (DESIGN item 20 rule 5): a process of the same user can write a record,
//! so a record is believed only while its process is alive, started when it says, is a `codex` program, and is an
//! ancestor of the caller. It grants no capability: `CodexRecorded` has exactly the levels of the generic floor and
//! only changes the name a join is shown under. Nothing here reads the environment, a Codex file or a credential.

use crate::harness::ancestry::{find_session_with, worth_visiting};
use crate::harness::liveness::{Liveness, check_with};
use crate::harness::process::{Lookup, ProcessTable};
use crate::harness::record::{FsRegistry, Record, Registry};
use std::path::{Path, PathBuf};

/// The program name of a Codex session. Not read from the environment: a different name is a code change.
pub const CODEX_PROGRAM: &str = "codex";

/// Whether `command` (as `ps -o comm=` prints it) is the Codex program: exactly the name, alone or at the end of a path.
/// Unlike Claude Code's, a retitled process (`codex something`) is not one, so a doubt is a miss.
pub fn is_codex(command: &str) -> bool {
    command.rsplit('/').next() == Some(CODEX_PROGRAM)
}

/// Where the records live under klatch's home directory.
pub fn codex_registry_dir(home: &Path) -> PathBuf {
    home.join("harness").join("codex")
}

/// Why no record was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotRecorded {
    /// No ancestor of the hook is a `codex` program.
    NoCodex,
    /// The process table could not be read.
    TableUnreadable,
    /// The directory or file could not be made safely.
    Unwritable,
}

/// What the hook knows of the session: its id (from its own standard input), directory and member name.
pub struct SessionFacts<'a> {
    pub session_id: &'a str,
    pub cwd: &'a str,
    pub name: Option<&'a str>,
}

/// The nearest process at or above `from` that is a `codex` program: its pid and start time. Bounded, never
/// revisits a pid, and any unreadable answer ends the walk with no process.
fn codex_above(from: u32, procs: &dyn ProcessTable) -> Result<(u32, String), NotRecorded> {
    let mut seen = Vec::new();
    let mut pid = from;
    while worth_visiting(pid, &seen) {
        seen.push(pid);
        match procs.lookup(pid) {
            Lookup::Found(info) if is_codex(&info.command) => return Ok((pid, info.start_utc)),
            Lookup::Found(info) => pid = info.ppid,
            Lookup::Missing => break,
            Lookup::Unreadable => return Err(NotRecorded::TableUnreadable),
        }
    }
    Err(NotRecorded::NoCodex)
}

/// The record's text, in the shape of a Claude Code registry record.
fn record_text(pid: u32, start: &str, domain: &str, facts: &SessionFacts) -> String {
    let mut v = serde_json::json!({
        "pid": pid,
        "sessionId": facts.session_id,
        "cwd": facts.cwd,
        "procStart": start,
        "pidDomain": domain,
    });
    if let Some(name) = facts.name {
        v["name"] = name.into();
    }
    v.to_string()
}

/// The registry directory, made if it is missing: never a symlink, always ours and mode 0700.
fn safe_dir(dir: &Path, uid: u32) -> Result<(), NotRecorded> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let bad = |_| NotRecorded::Unwritable;
    match std::fs::symlink_metadata(dir) {
        Ok(m) if !m.is_dir() => return Err(NotRecorded::Unwritable),
        Ok(m) if m.uid() != uid => return Err(NotRecorded::Unwritable),
        Ok(_) => {}
        Err(_) => std::fs::create_dir_all(dir).map_err(bad)?,
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(bad)
}

/// Remove the records of processes that are gone (dead, or the pid reused, or no longer a `codex`). A record that
/// cannot be judged (another pid domain, an unreadable table) is kept.
fn prune(dir: &Path, procs: &dyn ProcessTable, domain: &str) {
    for rec in (FsRegistry {
        dir: dir.to_path_buf(),
    })
    .records()
    .0
    {
        let gone = matches!(
            check_with(&rec, procs, domain, is_codex),
            Liveness::Dead | Liveness::Reused | Liveness::NotHarness
        );
        if gone {
            let _ = std::fs::remove_file(dir.join(format!("{}.json", rec.pid)));
        }
    }
}

/// Write the record of the Codex process above `from` (the hook's own pid) into `dir`. Nothing is written, not even the
/// directory, unless a `codex` process is found above the hook by a readable table.
pub fn write_session_record(
    dir: &Path,
    from: u32,
    facts: &SessionFacts,
    (procs, domain): (&dyn ProcessTable, &str),
) -> Result<u32, NotRecorded> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let (pid, start) = codex_above(from, procs)?;
    safe_dir(dir, unsafe { libc::geteuid() })?;
    prune(dir, procs, domain);
    let text = record_text(pid, &start, domain, facts);
    let temp = dir.join(format!(".{pid}.tmp{}", std::process::id()));
    let _ = std::fs::remove_file(&temp);
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)
        .and_then(|mut f| f.write_all(text.as_bytes()))
        .and_then(|()| std::fs::rename(&temp, dir.join(format!("{pid}.json"))));
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
        return Err(NotRecorded::Unwritable);
    }
    Ok(pid)
}

/// Whether the process `start` (or an ancestor) is the Codex process the live record of `session_id` names. The nearest
/// ancestor with a live record decides, and its record must be of this very session: a record of another session
/// is never evidence for this one. Any doubt is `false`.
pub fn verified_session(
    start: u32,
    session_id: &str,
    (procs, registry, domain): (&dyn ProcessTable, &dyn Registry, &str),
) -> bool {
    find_session_with(start, (procs, registry, domain), is_codex)
        .is_ok_and(|found| found.session_id == session_id)
}

/// The records of `dir`, read only when the file is a plain file owned by `uid` that no one else can write.
pub struct OwnRegistry {
    pub dir: PathBuf,
    pub uid: u32,
}

impl Registry for OwnRegistry {
    fn record_for(&self, pid: u32) -> Option<Record> {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let meta = std::fs::symlink_metadata(self.dir.join(format!("{pid}.json"))).ok()?;
        let plain = meta.is_file() && meta.uid() == self.uid;
        let sealed = meta.permissions().mode() & 0o022 == 0;
        (plain && sealed).then_some(())?;
        FsRegistry {
            dir: self.dir.clone(),
        }
        .record_for(pid)
    }
}

/// The harness of a join from a session: always the generic floor's levels, named `codex` only when `verify` says the
/// session runs in a recorded Codex process. `verify` runs at most once, and only when the name is asked for.
pub struct CodexRecorded {
    verify: Box<dyn Fn() -> bool>,
    verified: std::cell::OnceCell<bool>,
}

impl CodexRecorded {
    pub fn new(verify: Box<dyn Fn() -> bool>) -> CodexRecorded {
        CodexRecorded {
            verify,
            verified: std::cell::OnceCell::new(),
        }
    }
}

impl crate::harness::Harness for CodexRecorded {
    fn id(&self) -> &str {
        if *self.verified.get_or_init(|| (self.verify)()) {
            "codex"
        } else {
            "generic"
        }
    }
    fn level(&self, c: crate::harness::Capability) -> crate::harness::Level {
        crate::harness::Generic.level(c)
    }
    fn basis(&self, c: crate::harness::Capability) -> Option<&'static str> {
        crate::harness::Generic.basis(c)
    }
}

#[cfg(test)]
mod tests;
