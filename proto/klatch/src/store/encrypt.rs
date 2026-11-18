//! Moving a plaintext database to the encrypted format, in place (#164, R39).
//!
//! The copy is made beside the original and checked (every table's row count, the schema version and SQLCipher's
//! page check) before anything is replaced. The swap keeps the plaintext reachable under a second name until the
//! encrypted file is in place, then overwrites it with zeros and removes it, so a crash at any step loses nothing and
//! the next run finishes the job. The database file itself is the lock (`flock`), so no lock file is left behind and
//! several processes converting at once convert once.

use crate::domain::{Error, Result};
use crate::keys::Key;
use rusqlite::{Connection, params};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// What `ensure_encrypted` did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// There was nothing to do: no file, or it is already encrypted.
    Nothing,
    /// A plaintext copy left by an interrupted run was overwritten and removed.
    Scrubbed,
    /// A plaintext database was converted.
    Encrypted { tables: usize, rows: u64 },
}

const PLAIN_HEADER: &[u8; 16] = b"SQLite format 3\0";

fn io(what: &str, e: impl std::fmt::Display) -> Error {
    Error::Backend(format!("encrypting the store: {what}: {e}"))
}

fn sidecar(path: &Path, ext: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(ext);
    PathBuf::from(s)
}

/// Whether the file starts with the plain SQLite header: an encrypted file never does.
pub fn is_plaintext(path: &Path) -> bool {
    let mut head = [0u8; 16];
    File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && &head == PLAIN_HEADER
}

/// Overwrite every byte of the file with zeros, flush it to disk, then remove it. Best effort on media that move
/// data around (flash, copy-on-write file systems): it removes the easy copy, not forensic ones.
pub fn scrub_and_remove(path: &Path) -> Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|e| io("opening the old copy", e))?;
    let len = f
        .metadata()
        .map_err(|e| io("sizing the old copy", e))?
        .len();
    f.seek(SeekFrom::Start(0)).map_err(|e| io("seeking", e))?;
    let zeros = [0u8; 8192];
    let mut left = len;
    while left > 0 {
        let n = left.min(zeros.len() as u64) as usize;
        f.write_all(&zeros[..n]).map_err(|e| io("overwriting", e))?;
        left -= n as u64;
    }
    f.sync_all().map_err(|e| io("flushing", e))?;
    drop(f);
    std::fs::remove_file(path).map_err(|e| io("removing the old copy", e))
}

/// Take the exclusive lock on the directory that holds the database. The database file itself cannot carry it: on some
/// systems `flock` and the locks SQLite takes on the file conflict. The directory is not replaced by the swap, so one
/// lock covers the whole conversion, and nothing is left behind.
pub fn lock_dir(path: &Path) -> Result<File> {
    let f = open_dir(path)?;
    f.lock().map_err(|e| io("locking the store directory", e))?;
    Ok(f)
}

/// The shared lock on the same directory: many openers hold it together, and only a conversion or key rotation (the
/// exclusive lock) is kept apart from them.
pub fn lock_dir_shared(path: &Path) -> Result<File> {
    let f = open_dir(path)?;
    f.lock_shared()
        .map_err(|e| io("locking the store directory", e))?;
    Ok(f)
}

fn open_dir(path: &Path) -> Result<File> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    File::open(dir).map_err(|e| io("opening the store directory", e))
}

/// Bring the store at `path` to the encrypted format if it is not there yet, and remove any plaintext a previous run
/// left. Cheap when there is nothing to do (a file header read and two `stat`s), so it runs before every open.
pub fn ensure_encrypted(path: &Path, key: &Key) -> Result<Outcome> {
    let (enc, old) = (sidecar(path, ".encrypting"), sidecar(path, ".old"));
    let pending = enc.exists() || old.exists();
    if !pending && !is_plaintext(path) {
        return Ok(Outcome::Nothing);
    }
    let _lock = lock_dir(path)?;
    // under the lock nobody else is writing the copy: a leftover one is from a run that died
    let _ = std::fs::remove_file(&enc);
    if !path.exists() {
        // no database: only a half-written copy could be left, and it held nothing
        return Ok(Outcome::Nothing);
    }
    if is_plaintext(path) {
        // the plaintext stays locked against every other process until the swap and the scrub are done (#373)
        let (tables, rows, held) = convert(path, &enc, key)?;
        swap(path, &enc, &old)?;
        scrub_and_remove(&old)?;
        drop(held);
        return Ok(Outcome::Encrypted { tables, rows });
    }
    if old.exists() {
        scrub_and_remove(&old)?;
        return Ok(Outcome::Scrubbed);
    }
    Ok(Outcome::Nothing)
}

/// How many times a copy is made again because another process wrote to the plaintext between the copy and the lock
/// that holds it still.
const COPY_ATTEMPTS: usize = 5;

/// Copy the plaintext database into a new encrypted file and check the copy. Returns the table and row counts and
/// the connection that still holds the plaintext locked: an old klatch (plaintext, no directory lock) that writes
/// between the copy and the swap would lose that write, and one that is still writing after the swap would
/// write beside the new file, so nothing may write until the caller has swapped and dropped the guard (#373).
fn convert(path: &Path, enc: &Path, key: &Key) -> Result<(usize, u64, Connection)> {
    for _ in 0..COPY_ATTEMPTS {
        if let Some(done) = copy_once(path, enc, key)? {
            return Ok(done);
        }
        // someone wrote in the gap: the copy is stale, make it again
        let _ = std::fs::remove_file(enc);
    }
    Err(io(
        "copying",
        "the store kept being written to by another klatch; stop it and run this again",
    ))
}

/// How long the plaintext may stay busy (another klatch has it open) before the conversion gives up.
#[cfg(not(test))]
const BUSY_WAIT: Duration = Duration::from_secs(5);
#[cfg(test)]
const BUSY_WAIT: Duration = Duration::from_millis(300);

/// A test seam: code to run in the instant between the copy being committed and the plaintext being locked again,
/// which is where an old klatch's write would otherwise be lost (#373).
#[cfg(test)]
pub(crate) mod between_copy_and_lock {
    use std::cell::RefCell;

    type Hook = Box<dyn FnMut()>;

    thread_local! {
        static HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) };
    }

    pub(crate) fn set(f: impl FnMut() + 'static) {
        HOOK.with(|h| *h.borrow_mut() = Some(Box::new(f)));
    }

    pub(super) fn run() {
        HOOK.with(|h| {
            if let Some(f) = h.borrow_mut().as_mut() {
                f();
            }
        });
    }
}

/// Switch the plaintext database to rollback-journal mode. SQLite refuses while any other connection is open and
/// does not always wait for it, so this asks again every 20 ms for `BUSY_WAIT`: an old klatch that opens and closes
/// the store for each command leaves gaps, one that keeps it open (a long-lived process) does not, and then the
/// conversion stops with a message instead of going on under it.
fn leave_write_ahead_mode(conn: &Connection) -> Result<()> {
    let started = std::time::Instant::now();
    loop {
        let said = conn.query_row("PRAGMA journal_mode=DELETE", [], |r| r.get::<_, String>(0));
        match mode_switch(said) {
            Switch::Done => return Ok(()),
            Switch::Failed(e) => return Err(io("copying", e)),
            Switch::Wait => {}
        }
        if started.elapsed() >= BUSY_WAIT {
            return Err(io(
                "copying",
                "another klatch has the store open; stop it and run this again",
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// What SQLite's answer to the journal mode change means.
#[derive(Debug)]
enum Switch {
    /// The database is in rollback-journal mode now.
    Done,
    /// Another connection is in the way: SQLite said busy or locked, or kept the old mode (which it does, without
    /// an error, when it cannot change it). Ask again.
    Wait,
    /// Anything else is not going to get better by waiting.
    Failed(rusqlite::Error),
}

fn mode_switch(said: rusqlite::Result<String>) -> Switch {
    match said {
        Ok(mode) if mode.eq_ignore_ascii_case("delete") => Switch::Done,
        Ok(_) => Switch::Wait,
        Err(e) if is_busy(&e) => Switch::Wait,
        Err(e) => Switch::Failed(e),
    }
}

/// Whether SQLite said the database is busy or locked by another connection.
fn is_busy(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(f, _)
            if matches!(
                f.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            )
    )
}

/// One copy. `None` when the plaintext changed between the end of the copy and the lock taken again.
fn copy_once(path: &Path, enc: &Path, key: &Key) -> Result<Option<(usize, u64, Connection)>> {
    let sql = |e: rusqlite::Error| io("copying", e);
    let conn = Connection::open(path).map_err(sql)?;
    conn.busy_timeout(Duration::from_secs(5)).map_err(sql)?;
    // Rollback-journal mode: a lock then keeps every other process out, readers too, and closing the connection
    // touches no file by name. Leaving write-ahead mode needs every other connection closed: a store another
    // klatch still holds open is refused (after the wait) instead of being converted under it.
    leave_write_ahead_mode(&conn)?;
    conn.execute_batch("BEGIN EXCLUSIVE").map_err(sql)?;
    let before: i64 = conn
        .query_row("PRAGMA data_version", [], |r| r.get(0))
        .map_err(sql)?;
    let pragma = key.sqlcipher_pragma();
    conn.execute(
        "ATTACH DATABASE ?1 AS enc KEY ?2",
        params![enc.to_string_lossy(), pragma.as_str()],
    )
    .map_err(sql)?;
    conn.query_row("SELECT sqlcipher_export('enc')", [], |_| Ok(()))
        .map_err(sql)?;
    let version: i64 = conn
        .query_row("PRAGMA main.user_version", [], |r| r.get(0))
        .map_err(sql)?;
    conn.execute_batch(&format!("PRAGMA enc.user_version = {version}"))
        .map_err(sql)?;
    let tables: Vec<String> = conn
        .prepare(
            "SELECT name FROM main.sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
        )
        .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
        .map_err(sql)?;
    let mut rows = 0u64;
    for t in &tables {
        let q = |db: &str| -> rusqlite::Result<i64> {
            conn.query_row(&format!("SELECT count(*) FROM {db}.\"{t}\""), [], |r| {
                r.get(0)
            })
        };
        let (a, b) = (q("main").map_err(sql)?, q("enc").map_err(sql)?);
        if a != b {
            return Err(io(
                "verifying",
                format!("table {t} has {a} rows and its copy {b}"),
            ));
        }
        rows += a as u64;
    }
    conn.execute_batch("COMMIT").map_err(sql)?;
    // (detached, so the exclusive lock below covers the plaintext only and the copy can be opened to verify it)
    conn.execute_batch("DETACH DATABASE enc").map_err(sql)?;
    #[cfg(test)]
    between_copy_and_lock::run();
    // the copy is on disk and the plaintext lock was let go for an instant: take it again and see whether anyone
    // wrote in between; if so the copy is stale
    conn.execute_batch("BEGIN EXCLUSIVE").map_err(sql)?;
    let after: i64 = conn
        .query_row("PRAGMA data_version", [], |r| r.get(0))
        .map_err(sql)?;
    if after != before {
        let _ = conn.execute_batch("ROLLBACK");
        return Ok(None);
    }
    verify_copy(enc, key, version, tables.len())?;
    Ok(Some((tables.len(), rows, conn)))
}

/// Open the finished copy as a stranger would and check it: the key opens it, the version matches, SQLCipher's page
/// check finds nothing wrong.
fn verify_copy(enc: &Path, key: &Key, version: i64, tables: usize) -> Result<()> {
    let sql = |e: rusqlite::Error| io("verifying the copy", e);
    let c = Connection::open(enc).map_err(sql)?;
    c.pragma_update(None, "key", key.sqlcipher_pragma().as_str())
        .map_err(sql)?;
    let n: i64 = c
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            [],
            |r| r.get(0),
        )
        .map_err(sql)?;
    let v: i64 = c
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(sql)?;
    let problems: Vec<String> = c
        .prepare("PRAGMA cipher_integrity_check")
        .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
        .map_err(sql)?;
    let same_shape = n as usize == tables && v == version;
    if !same_shape || !problems.is_empty() {
        return Err(io(
            "verifying the copy",
            format!(
                "{n} tables of {tables}, version {v} of {version}, {} page problems",
                problems.len()
            ),
        ));
    }
    Ok(())
}

/// Scrub and remove `side` if it is there. An old klatch closing the database removes its own log files, so a file
/// that vanishes while this works on it is what was wanted, not a failure.
fn scrub_if_present(side: &Path) -> Result<()> {
    if !side.exists() {
        return Ok(());
    }
    match scrub_and_remove(side) {
        Err(_) if !side.exists() => Ok(()),
        done => done,
    }
}

/// Put the encrypted copy in place. The plaintext stays reachable as `old` until the new file is the database, so a
/// crash here loses nothing; its write-ahead log and shared memory file go first, because a stale log beside the new
/// file would be applied to it.
fn swap(path: &Path, enc: &Path, old: &Path) -> Result<()> {
    for ext in ["-wal", "-shm"] {
        let side = sidecar(path, ext);
        scrub_if_present(&side)?;
    }
    std::fs::hard_link(path, old).map_err(|e| io("keeping the old copy", e))?;
    std::fs::rename(enc, path).map_err(|e| io("putting the copy in place", e))
}

/// Whether the encrypted file at `path` opens with `key` and passes SQLCipher's page check. Changes nothing.
pub(crate) fn check_opens(path: &Path, key: &Key) -> Result<()> {
    let sql = |e: rusqlite::Error| io("checking the copy", e);
    let c = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(sql)?;
    c.pragma_update(None, "key", key.sqlcipher_pragma().as_str())
        .map_err(|e| io("checking the copy", e))?;
    c.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })
    .map_err(|_| io("checking the copy", "it does not open with this key"))?;
    let problems: Vec<String> = c
        .prepare("PRAGMA cipher_integrity_check")
        .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
        .map_err(sql)?;
    if problems.is_empty() {
        Ok(())
    } else {
        Err(io(
            "checking the copy",
            format!("{} damaged pages", problems.len()),
        ))
    }
}

/// Replace the store at `db` with the backup at `backup`. The backup is checked first (it must open with `key`);
/// the store it replaces is folded into one file and kept beside it as `<db>.before-restore`, still encrypted, so a
/// mistaken restore can be undone. A refused restore changes nothing.
pub fn restore_backup(db: &Path, backup: &Path, key: &Key) -> Result<()> {
    let restoring = sidecar(db, ".restoring");
    let aside = sidecar(db, ".before-restore");
    let _lock = lock_dir(db)?;
    let _ = std::fs::remove_file(&restoring);
    let outcome = (|| -> Result<()> {
        std::fs::copy(backup, &restoring).map_err(|e| io("reading the backup", e))?;
        check_opens(&restoring, key)?;
        if db.exists() {
            // the current store must open with this key, and its log is folded in so the kept copy is whole
            let c = Connection::open(db).map_err(|e| io("opening the current store", e))?;
            c.pragma_update(None, "key", key.sqlcipher_pragma().as_str())
                .map_err(|e| io("opening the current store", e))?;
            c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
                .map_err(|e| io("opening the current store (is the key right?)", e))?;
            drop(c);
            for ext in ["-wal", "-shm"] {
                let _ = std::fs::remove_file(sidecar(db, ext));
            }
            let _ = std::fs::remove_file(&aside);
            std::fs::rename(db, &aside).map_err(|e| io("keeping the current store", e))?;
        }
        std::fs::rename(&restoring, db).map_err(|e| io("putting the backup in place", e))
    })();
    if outcome.is_err() {
        let _ = std::fs::remove_file(&restoring);
    }
    outcome
}

#[cfg(test)]
mod tests;
