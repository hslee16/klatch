//! Retention and archive (#69, #70). Old messages of a room move into an encrypted archive file and leave a tombstone
//! behind (same room, seq, id, sender and time; the text replaced), so `seq` stays gapless and a reference to the
//! message still resolves. A restore puts the text back from the archive. The archive is written under the store key
//! with SQLCipher, so no plaintext copy of old messages exists at any point; sealed-label text is copied as the
//! ciphertext it is stored as.

use super::{SqliteStore, be};
use crate::domain::{Error, Result};
use crate::keys::Key;
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

/// What a tombstone says in place of the text.
pub const TOMBSTONE: &str = "[archived: restore it with `klatch admin restore-archive`]";
const KIND: &str = "archived";

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
        let due: i64 = self
            .conn
            .query_row(
                "SELECT count(*) FROM messages WHERE room=?1 AND ts<?2 AND COALESCE(kind,'')<>?3",
                params![room, cutoff, KIND],
                |r| r.get(0),
            )
            .map_err(be)?;
        if due == 0 {
            return Ok(0);
        }
        if out.exists() {
            return Err(Error::Invalid(format!(
                "{} already exists; an archive never overwrites a file",
                out.display()
            )));
        }
        // one partial file per process: two archivers that share a target name (the name carries only seconds) must
        // not remove or reuse each other's file (#598)
        let mut partial = out.as_os_str().to_owned();
        partial.push(format!(".{}.partial", std::process::id()));
        let partial = std::path::PathBuf::from(partial);
        let _ = std::fs::remove_file(&partial);
        let copied = (|| -> Result<usize> {
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
            self.conn.execute_batch("DETACH DATABASE ar").map_err(be)?;
            crate::store::encrypt::check_opens(&partial, key)?;
            // a hard link fails when the target exists, where a rename would silently replace another archiver's file
            let linked = std::fs::hard_link(&partial, out);
            let _ = std::fs::remove_file(&partial);
            linked.map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    Error::Invalid(format!(
                        "{} already exists; an archive never overwrites a file",
                        out.display()
                    ))
                } else {
                    Error::Backend(format!("putting the archive in place: {e}"))
                }
            })?;
            Ok(n)
        })();
        let copied = match copied {
            Ok(n) => n,
            Err(e) => {
                let _ = self.conn.execute_batch("DETACH DATABASE ar");
                let _ = std::fs::remove_file(&partial);
                return Err(e);
            }
        };
        self.immediate(|| {
            self.exec(
                "UPDATE messages SET body=?4, payload=NULL, kind=?3 WHERE room=?1 AND ts<?2 AND COALESCE(kind,'')<>?3",
                params![room, cutoff, KIND, TOMBSTONE],
            )
            .map(|_| ())
        })?;
        Ok(copied)
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
        let result = self.immediate(|| {
            self.exec(
                "UPDATE main.messages SET
                   body=(SELECT a.body FROM ar.archived a WHERE a.room=messages.room AND a.seq=messages.seq),
                   payload=(SELECT a.payload FROM ar.archived a WHERE a.room=messages.room AND a.seq=messages.seq),
                   kind=(SELECT a.kind FROM ar.archived a WHERE a.room=messages.room AND a.seq=messages.seq)
                 WHERE kind=?1 AND body=?2 AND EXISTS
                   (SELECT 1 FROM ar.archived a WHERE a.room=messages.room AND a.seq=messages.seq AND a.mid=messages.mid)",
                params![KIND, TOMBSTONE],
            )
        });
        let _ = self.conn.execute_batch("DETACH DATABASE ar");
        result
    }
}
