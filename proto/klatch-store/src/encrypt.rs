//! Moving a plaintext database to the encrypted format, in place (#164, R39).
//!
//! The copy is made beside the original and checked (every table's row count, the schema version and SQLCipher's
//! page check) before anything is replaced. The swap keeps the plaintext reachable under a second name until the
//! encrypted file is in place, then overwrites it with zeros and removes it, so a crash at any step loses nothing and
//! the next run finishes the job. The database file itself is the lock (`flock`), so no lock file is left behind and
//! several processes converting at once convert once.

use klatch_core::{Error, Result};
use klatch_keys::Key;
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
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let f = File::open(dir).map_err(|e| io("opening the store directory", e))?;
    f.lock().map_err(|e| io("locking the store directory", e))?;
    Ok(f)
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
        let (tables, rows) = convert(path, &enc, key)?;
        swap(path, &enc, &old)?;
        scrub_and_remove(&old)?;
        return Ok(Outcome::Encrypted { tables, rows });
    }
    if old.exists() {
        scrub_and_remove(&old)?;
        return Ok(Outcome::Scrubbed);
    }
    Ok(Outcome::Nothing)
}

/// Copy the plaintext database into a new encrypted file and check the copy. Returns the table and row counts.
fn convert(path: &Path, enc: &Path, key: &Key) -> Result<(usize, u64)> {
    let sql = |e: rusqlite::Error| io("copying", e);
    let conn = Connection::open(path).map_err(sql)?;
    conn.busy_timeout(Duration::from_secs(5)).map_err(sql)?;
    // fold the write-ahead log into the file so the copy sees everything
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .map_err(sql)?;
    conn.execute_batch("BEGIN EXCLUSIVE").map_err(sql)?;
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
    drop(conn);
    verify_copy(enc, key, version, tables.len())?;
    Ok((tables.len(), rows))
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
    if n as usize != tables || v != version || !problems.is_empty() {
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

/// Put the encrypted copy in place. The plaintext stays reachable as `old` until the new file is the database, so a
/// crash here loses nothing; its write-ahead log and shared memory file go first, because a stale log beside the new
/// file would be applied to it.
fn swap(path: &Path, enc: &Path, old: &Path) -> Result<()> {
    for ext in ["-wal", "-shm"] {
        let side = sidecar(path, ext);
        if side.exists() {
            scrub_and_remove(&side)?;
        }
    }
    std::fs::hard_link(path, old).map_err(|e| io("keeping the old copy", e))?;
    std::fs::rename(enc, path).map_err(|e| io("putting the copy in place", e))
}
