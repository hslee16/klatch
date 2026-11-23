//! Retention and archive (#69, #70). Old messages of a room move into an encrypted archive file and leave a tombstone
//! behind (same room, seq, id, sender and time; the text replaced), so `seq` stays gapless and a reference to the
//! message still resolves. A restore puts the text back from the archive. The archive is written under the store key
//! with SQLCipher, so no plaintext copy of old messages exists at any point; sealed-label text is copied as the
//! ciphertext it is stored as.

use super::{SqliteStore, be};
use crate::domain::{Error, Result};
use crate::keys::Key;
use crate::store::PolicyStore;
use rusqlite::params;
use std::path::{Path, PathBuf};

/// What a check of one file under the archive directory found (#487).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveState {
    /// Opens with the store key, every page passes SQLCipher's check, and holds an `archived` table.
    Ok {
        rooms: Vec<String>,
        messages: usize,
        oldest: i64,
        newest: i64,
    },
    /// Present but not usable, with why (wrong key, damaged pages, not encrypted, not an archive).
    Unreadable(String),
    /// A `.db.partial` left by an `archive` that was interrupted; never part of a restore, safe to remove.
    Partial,
}

/// One file under the archive directory: counts only, never message text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveInfo {
    pub path: PathBuf,
    pub bytes: u64,
    /// Last modification, unix seconds.
    pub modified: Option<i64>,
    pub state: ArchiveState,
}

/// The archive files in `dir` in name order, each opened with `key` and checked. A missing directory has none. Files
/// that are neither `*.db` nor `*.db.partial` are not ours and are left out. Reads nothing but counts, changes nothing.
pub fn list_archives(dir: &Path, key: &Key) -> Result<Vec<ArchiveInfo>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::Backend(format!("{}: {e}", dir.display()))),
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    let mut found = Vec::new();
    for path in paths {
        let name = path.to_string_lossy();
        let state = if name.ends_with(".db.partial") {
            ArchiveState::Partial
        } else if name.ends_with(".db") {
            inspect(&path, key)
        } else {
            continue;
        };
        let meta = std::fs::metadata(&path).ok();
        found.push(ArchiveInfo {
            bytes: meta.as_ref().map_or(0, |m| m.len()),
            modified: meta
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64),
            path,
            state,
        });
    }
    Ok(found)
}

fn inspect(path: &Path, key: &Key) -> ArchiveState {
    if crate::store::encrypt::is_plaintext(path) {
        return ArchiveState::Unreadable(
            "not encrypted (an archive always is; this file does not belong here)".to_string(),
        );
    }
    if let Err(e) = crate::store::encrypt::check_opens(path, key) {
        return ArchiveState::Unreadable(e.to_string());
    }
    summarize(path, key)
        .unwrap_or_else(|e| ArchiveState::Unreadable(format!("not an archive: {e}")))
}

/// Rooms, count and the oldest and newest message time of a file that already opened and passed the page check.
fn summarize(path: &Path, key: &Key) -> rusqlite::Result<ArchiveState> {
    let c =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    c.pragma_update(None, "key", key.sqlcipher_pragma().as_str())?;
    let (messages, oldest, newest): (i64, Option<i64>, Option<i64>) =
        c.query_row("SELECT count(*), min(ts), max(ts) FROM archived", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
    let rooms = c
        .prepare("SELECT DISTINCT room FROM archived ORDER BY room")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(ArchiveState::Ok {
        rooms,
        messages: messages as usize,
        oldest: oldest.unwrap_or(0),
        newest: newest.unwrap_or(0),
    })
}

/// Move every archive file in `dir` from `old` to `new` (#628), so a store key rotation does not strand archived
/// messages. Returns how many files were moved. Safe to repeat: a file that already opens with `new` is skipped, so a
/// run that died half way is finished by the next. Nothing changes unless every `*.db` file opens with one of the two
/// keys (a file neither opens would be lost to the rotation, so the caller must hear of it first).
pub fn rekey_archives(dir: &Path, old: &Key, new: &Key) -> Result<usize> {
    rekey_archives_hooked(dir, old, new, &|_| Ok(()))
}

/// `rekey_archives` with a hook called after each file is moved, with the count so far; an error from it stops the
/// run there. A test uses it to stand in for a crash between two files.
pub(crate) fn rekey_archives_hooked(
    dir: &Path,
    old: &Key,
    new: &Key,
    after_each: &dyn Fn(usize) -> Result<()>,
) -> Result<usize> {
    let todo = archives_to_rekey(dir, old, new)?;
    for (done, path) in todo.iter().enumerate() {
        rekey_archives_in(path, old, new)?;
        after_each(done + 1)?;
    }
    Ok(todo.len())
}

/// The archive files that still open with `old` (those that open with `new` are done). An error, with every
/// offending file named, when any file opens with neither; nothing is changed by this call, so a caller can refuse a
/// key rotation before it has stored or used the new key.
pub fn archives_to_rekey(dir: &Path, old: &Key, new: &Key) -> Result<Vec<PathBuf>> {
    let mut todo = Vec::new();
    let mut stuck = Vec::new();
    for path in archive_files(dir)? {
        if crate::store::encrypt::check_opens(&path, new).is_ok() {
            continue;
        }
        match crate::store::encrypt::check_opens(&path, old) {
            Ok(()) => todo.push(path),
            Err(e) => stuck.push(format!("{}: {e}", path.display())),
        }
    }
    if !stuck.is_empty() {
        return Err(Error::Invalid(format!(
            "{} archive file(s) open with neither the current key nor the new one, so the key was not changed (restore them from a backup or move them out of the archive directory): {}",
            stuck.len(),
            stuck.join("; ")
        )));
    }
    Ok(todo)
}

/// The `*.db` files directly in `dir`, in name order. A missing directory has none.
pub(super) fn archive_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::Backend(format!("{}: {e}", dir.display()))),
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().ends_with(".db"))
        .collect();
    files.sort();
    Ok(files)
}

/// Write `file` again under `new` and put the copy in its place. The copy is made beside it, opened and counted
/// before anything is replaced, and the replacement is a rename, so at every moment the name holds a whole archive
/// under `old` or under `new` and a crash leaves at worst a `.partial` file that `admin archives` calls safe to remove.
pub fn rekey_archives_in(file: &Path, old: &Key, new: &Key) -> Result<()> {
    let mut partial = file.as_os_str().to_owned();
    partial.push(format!(".{}.partial", std::process::id()));
    let partial = PathBuf::from(partial);
    let _ = std::fs::remove_file(&partial);
    let made = copy_rekeyed(file, &partial, old, new);
    let placed = made.and_then(|()| {
        std::fs::rename(&partial, file)
            .map_err(|e| Error::Backend(format!("putting {} in place: {e}", file.display())))
    });
    if placed.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    placed
}

fn copy_rekeyed(file: &Path, partial: &Path, old: &Key, new: &Key) -> Result<()> {
    let conn = rusqlite::Connection::open(file).map_err(be)?;
    conn.pragma_update(None, "key", old.sqlcipher_pragma().as_str())
        .map_err(be)?;
    conn.execute(
        "ATTACH DATABASE ?1 AS rk KEY ?2",
        params![partial.to_string_lossy(), new.sqlcipher_pragma().as_str()],
    )
    .map_err(be)?;
    conn.query_row("SELECT sqlcipher_export('rk')", [], |_| Ok(()))
        .map_err(be)?;
    let version: i64 = conn
        .query_row("PRAGMA main.user_version", [], |r| r.get(0))
        .map_err(be)?;
    conn.execute_batch(&format!("PRAGMA rk.user_version = {version}"))
        .map_err(be)?;
    let count = |db: &str| -> Result<i64> {
        conn.query_row(&format!("SELECT count(*) FROM {db}.archived"), [], |r| {
            r.get(0)
        })
        .map_err(be)
    };
    let (before, after) = (count("main")?, count("rk")?);
    conn.execute_batch("DETACH DATABASE rk").map_err(be)?;
    drop(conn);
    if before != after {
        return Err(Error::Backend(format!(
            "re-keying {}: {before} messages and its copy {after}",
            file.display()
        )));
    }
    crate::store::encrypt::check_opens(partial, new)
}

/// What a tombstone says in place of the text.
pub const TOMBSTONE: &str = "[archived: restore it with `klatch admin restore-archive`]";
const KIND: &str = "archived";

/// One archive run: the room, the cutoff (unix seconds), the archive file to write and the key it is written under.
pub(crate) struct ArchiveJob<'a> {
    pub(crate) room: &'a str,
    pub(crate) cutoff: i64,
    pub(crate) out: &'a Path,
    pub(crate) key: &'a Key,
}

fn already_exists(out: &Path) -> Error {
    Error::Invalid(format!(
        "{} already exists; an archive never overwrites a file",
        out.display()
    ))
}

impl SqliteStore {
    /// Every room that has at least one message.
    pub fn message_rooms(&self) -> Result<Vec<String>> {
        self.rows(
            "SELECT DISTINCT room FROM messages ORDER BY room",
            [],
            |r| r.get(0),
        )
    }

    /// Move the messages of `room` older than `cutoff` (unix seconds) into a new encrypted archive file `out` and
    /// leave tombstones. Returns how many moved; with none, no file is written. Never overwrites a file, builds the
    /// archive beside the target and checks it opens before putting it in place, and changes the live rows only after
    /// the archive is safely there.
    pub fn archive_before(&self, room: &str, cutoff: i64, out: &Path, key: &Key) -> Result<usize> {
        let job = ArchiveJob {
            room,
            cutoff,
            out,
            key,
        };
        self.archive_before_hooked(&job, &|| {})
    }

    /// `archive_before` with a hook that runs after the copy and before the tombstones: a test lands a message in
    /// exactly that window (#634). Production passes a no-op.
    pub(crate) fn archive_before_hooked(
        &self,
        job: &ArchiveJob<'_>,
        after_copy: &dyn Fn(),
    ) -> Result<usize> {
        let due: i64 = self
            .conn
            .query_row(
                "SELECT count(*) FROM messages WHERE room=?1 AND ts<?2 AND COALESCE(kind,'')<>?3",
                params![job.room, job.cutoff, KIND],
                |r| r.get(0),
            )
            .map_err(be)?;
        if due == 0 {
            return Ok(0);
        }
        if job.out.exists() {
            return Err(already_exists(job.out));
        }
        // one partial file per process: two archivers that share a target name (the name carries only seconds) must
        // not remove or reuse each other's file (#598)
        let mut partial = job.out.as_os_str().to_owned();
        partial.push(format!(".{}.partial", std::process::id()));
        let partial = std::path::PathBuf::from(partial);
        let _ = std::fs::remove_file(&partial);
        let copied = match self.copy_due_rows(job, &partial) {
            Ok(n) => n,
            Err(e) => {
                let _ = self.conn.execute_batch("DETACH DATABASE ar");
                let _ = self.conn.execute_batch("DROP TABLE IF EXISTS temp.copied");
                let _ = std::fs::remove_file(&partial);
                return Err(e);
            }
        };
        after_copy();
        let tombstoned = self.tombstone_copied(job);
        let _ = self.conn.execute_batch("DROP TABLE IF EXISTS temp.copied");
        tombstoned?;
        Ok(copied)
    }

    /// Copy the due rows of the job into the encrypted `partial` file, remember which went in, and put the archive in
    /// place under its name without replacing anything. Returns how many rows were copied.
    fn copy_due_rows(&self, job: &ArchiveJob<'_>, partial: &Path) -> Result<usize> {
        let (room, cutoff, key) = (job.room, job.cutoff, job.key);
        self.conn
            .execute(
                "ATTACH DATABASE ?1 AS ar KEY ?2",
                params![partial.to_string_lossy(), key.sqlcipher_pragma().as_str()],
            )
            .map_err(be)?;
        self.conn
            .execute_batch(
                "CREATE TABLE ar.archived(room TEXT NOT NULL, seq INTEGER NOT NULL, mid TEXT NOT NULL, sender TEXT, member TEXT, body TEXT, ts INTEGER, attribution TEXT NOT NULL, label TEXT NOT NULL, kind TEXT, payload TEXT, PRIMARY KEY(room,seq));",
            )
            .map_err(be)?;
        let n = self
            .conn
            .execute(
                "INSERT INTO ar.archived SELECT room,seq,mid,sender,member,body,ts,attribution,label,kind,payload FROM main.messages WHERE room=?1 AND ts<?2 AND COALESCE(kind,'')<>?3",
                params![room, cutoff, KIND],
            )
            .map_err(be)?;
        // remember exactly which rows went into the file; the tombstones go on these and no others (#634). The
        // table has a primary key because an unkeyed one made the tombstone UPDATE scan it once per live row:
        // minutes at fifty thousand messages, with the write lock held.
        self.conn
            .execute_batch(
                "DROP TABLE IF EXISTS temp.copied;
                 CREATE TEMP TABLE copied(room TEXT NOT NULL, seq INTEGER NOT NULL, mid TEXT NOT NULL, PRIMARY KEY(room, seq)) WITHOUT ROWID;
                 INSERT INTO temp.copied SELECT room, seq, mid FROM ar.archived;",
            )
            .map_err(be)?;
        self.conn.execute_batch("DETACH DATABASE ar").map_err(be)?;
        crate::store::encrypt::check_opens(partial, key)?;
        // a hard link fails when the target exists, where a rename would silently replace another archiver's file
        let linked = std::fs::hard_link(partial, job.out);
        let _ = std::fs::remove_file(partial);
        linked.map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                already_exists(job.out)
            } else {
                Error::Backend(format!("putting the archive in place: {e}"))
            }
        })?;
        Ok(n)
    }

    /// Replace the text of exactly the rows that were copied (the `temp.copied` table) with the tombstone.
    fn tombstone_copied(&self, job: &ArchiveJob<'_>) -> Result<()> {
        self.immediate(|| {
            self.exec(
                "UPDATE messages SET body=?4, payload=NULL, kind=?3 WHERE room=?1 AND ts<?2 AND COALESCE(kind,'')<>?3
                   AND EXISTS (SELECT 1 FROM temp.copied c WHERE c.room=messages.room AND c.seq=messages.seq AND c.mid=messages.mid)",
                params![job.room, job.cutoff, KIND, TOMBSTONE],
            )
            .map(|_| ())
        })
    }

    /// Put archived messages back from `file`: a live row is restored only when it is a tombstone with the same room,
    /// seq and id, so an archive from another store or an edited row changes nothing. Returns how many came back; the
    /// archive file is kept.
    pub fn restore_archive(&self, file: &Path, key: &Key) -> Result<usize> {
        crate::store::encrypt::check_opens(file, key)?;
        self.conn
            .execute(
                "ATTACH DATABASE ?1 AS ar KEY ?2",
                params![file.to_string_lossy(), key.sqlcipher_pragma().as_str()],
            )
            .map_err(be)?;
        let result = self.immediate(|| self.restore_rows());
        let _ = self.conn.execute_batch("DETACH DATABASE ar");
        result
    }

    /// Seal the plain texts (`body`, `payload`) of a restored message of a sealed label under the label key, or refuse
    /// when the key is not held.
    fn protect_restored(
        &self,
        mid: &str,
        label: &str,
        mut out: [Option<String>; 2],
    ) -> Result<[Option<String>; 2]> {
        let ring = self.labels.borrow();
        for (text, field) in out.iter_mut().zip(["body", "payload"]) {
            let Some(plain) = text.as_deref().filter(|t| !crate::keys::is_sealed_text(t)) else {
                continue;
            };
            let Some(key) = ring.get(label) else {
                return Err(Error::Refused {
                    rule: "label-key".into(),
                    reason: format!(
                        "the archive holds plain {label} text and label {label} is sealed, and you hold no key for it, so nothing was restored"
                    ),
                });
            };
            let ctx = Self::sealed_context(mid, label, field);
            *text = Some(
                crate::keys::protect_text(key, ctx.as_bytes(), plain)
                    .map_err(|e| Error::Backend(format!("no random source: {e}")))?,
            );
        }
        Ok(out)
    }

    /// The restore itself, with the archive attached as `ar`. Plain archived text of a label that is sealed now is
    /// sealed under the label key on the way back, and refused (with nothing changed) when the key is not held, so
    /// a restore never puts plain text under a sealed label (#1153).
    fn restore_rows(&self) -> Result<usize> {
        type Back = (i64, String, String, [Option<String>; 2], Option<String>);
        let rows: Vec<Back> = self.rows(
            "SELECT m.id, m.mid, m.label, a.body, a.payload, a.kind FROM main.messages m
               JOIN ar.archived a ON a.room=m.room AND a.seq=m.seq AND a.mid=m.mid
              WHERE m.kind=?1 AND m.body=?2",
            params![KIND, TOMBSTONE],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    [r.get(3)?, r.get(4)?],
                    r.get(5)?,
                ))
            },
        )?;
        let mut sealed_labels = std::collections::HashMap::new();
        let mut done = Vec::new();
        for (id, mid, label, texts, kind) in rows {
            if !sealed_labels.contains_key(&label) {
                let is_sealed = self.policy_get(&format!("sealed:{label}"))?.is_some();
                sealed_labels.insert(label.clone(), is_sealed);
            }
            let [body, payload] = if sealed_labels[&label] {
                self.protect_restored(&mid, &label, texts)?
            } else {
                texts
            };
            done.push((id, body, payload, kind));
        }
        let count = done.len();
        for (id, body, payload, kind) in done {
            self.exec(
                "UPDATE main.messages SET body=?2, payload=?3, kind=?4 WHERE id=?1",
                params![id, body, payload, kind],
            )?;
        }
        Ok(count)
    }
}
