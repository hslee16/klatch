//! Opening the encrypted store: the key, the pragmas, the schema check and the retry on contention.

use super::schema::{SCHEMA, has_table, user_version};
use super::{SCHEMA_VERSION, SqliteStore, schema};
use crate::domain::{Error, Result};
use crate::keys::Key;
use crate::store::Census;
use rusqlite::Connection;
use std::path::Path;
use std::time::Duration;

pub(super) fn be(e: rusqlite::Error) -> Error {
    match &e {
        rusqlite::Error::SqliteFailure(f, _)
            if matches!(
                f.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            ) =>
        {
            Error::Busy(e.to_string())
        }
        _ => Error::Backend(e.to_string()),
    }
}

impl SqliteStore {
    /// Open the encrypted store at `path` with `key`, with the default five second wait for another writer. There
    /// is no way to open a file without a key: the store is encrypted at rest (R32).
    pub fn open(path: &Path, key: &Key) -> Result<Self> {
        Self::open_with_timeout(path, key, Duration::from_secs(5))
    }

    /// Open with an explicit limit on how long a write waits for another writer. Past the limit the
    /// write fails with `Error::Backend` and leaves nothing behind; it never hangs.
    pub fn open_with_timeout(path: &Path, key: &Key, busy: Duration) -> Result<Self> {
        // Switching a fresh database to WAL and creating its schema can collide with other first-time
        // openers in a way SQLite's own busy handler does not cover, so the whole open is retried on
        // contention, with a small backoff, until the limit.
        let deadline = std::time::Instant::now() + busy;
        let mut attempt: u64 = 0;
        loop {
            match Self::try_open(path, key, busy) {
                Err(Error::Busy(_)) if std::time::Instant::now() < deadline => {
                    let step = (2u64 << attempt.min(4)).min(50);
                    let jitter = (u64::from(std::process::id()) * 7 + attempt * 13) % 5;
                    std::thread::sleep(Duration::from_millis(step + jitter));
                    attempt += 1;
                }
                other => return other,
            }
        }
    }

    /// Every member's last reported permission mode, read from the file without opening it for writing: no migration,
    /// no journal switch, nothing created (`klatch doctor` must change nothing). A store from before the table
    /// existed has no modes yet.
    /// Row counts of the tables that only grow: live messages (tombstones left by archiving are not counted), the
    /// audit log, refusals and claim notes. Read-only (#487); a table the store does not have yet counts as 0.
    pub fn read_growth(path: &Path, key: &Key) -> Result<Vec<(&'static str, i64)>> {
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(be)?;
        Self::unlock(&conn, key)?;
        let mut out = Vec::new();
        for (label, table, filter) in [
            ("messages", "messages", " WHERE kind IS NOT 'archived'"),
            ("audit", "admin_audit", ""),
            ("refusals", "refusals", ""),
            ("claim notes", "claim_notes", ""),
        ] {
            let has: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |r| r.get(0),
                )
                .map_err(be)?;
            let n = if has == 0 {
                0
            } else {
                conn.query_row(&format!("SELECT COUNT(*) FROM {table}{filter}"), [], |r| {
                    r.get(0)
                })
                .map_err(be)?
            };
            out.push((label, n));
        }
        Ok(out)
    }

    /// The count of `action` entries of the audit since `since`, and of the distinct terminals among them (#1310), for
    /// `klatch doctor`. Read-only; counts only.
    pub fn read_audit_census(path: &Path, key: &Key, q: &Census) -> Result<(u64, u64)> {
        use super::api::policy::{census_from_row, census_params, census_sql};
        if q.targets.is_empty() {
            return Ok((0, 0));
        }
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(be)?;
        Self::unlock(&conn, key)?;
        let has: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='admin_audit'",
                [],
                |r| r.get(0),
            )
            .map_err(be)?;
        if has == 0 {
            return Ok((0, 0));
        }
        conn.query_row(
            &census_sql(q.targets.len()),
            rusqlite::params_from_iter(census_params(q)),
            census_from_row,
        )
        .map_err(be)
    }

    /// Whether the policy table holds `key` (#1262), for `klatch doctor`: a read-only connection, one indexed lookup,
    /// and the value is never brought back.
    pub fn read_policy_has(path: &Path, key: &Key, policy_key: &str) -> Result<bool> {
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(be)?;
        Self::unlock(&conn, key)?;
        let has: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='policy'",
                [],
                |r| r.get(0),
            )
            .map_err(be)?;
        if has == 0 {
            return Ok(false);
        }
        conn.query_row(
            "SELECT COUNT(*) FROM policy WHERE key=?1",
            [policy_key],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .map_err(be)
    }

    pub fn read_member_modes(path: &Path, key: &Key) -> Result<Vec<(String, String, i64)>> {
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(be)?;
        Self::unlock(&conn, key)?;
        let has_table: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name='member_mode'",
                [],
                |r| r.get(0),
            )
            .map_err(be)?;
        if has_table == 0 {
            return Ok(Vec::new());
        }
        let mut stmt = conn
            .prepare("SELECT member,mode,ts FROM member_mode ORDER BY member")
            .map_err(be)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(be)?;
        rows.collect::<std::result::Result<_, _>>().map_err(be)
    }

    pub(super) fn try_open(path: &Path, key: &Key, busy: Duration) -> Result<Self> {
        let conn = Connection::open(path).map_err(be)?;
        Self::unlock(&conn, key)?;
        conn.busy_timeout(busy).map_err(be)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(be)?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(be)?;
        // sorting and indexing spill to memory, never to a temporary file (R37)
        conn.pragma_update(None, "temp_store", "MEMORY")
            .map_err(be)?;
        Self::init(conn)
    }

    /// Apply the key and check it: this must be the first thing done on a connection. Refuses a build that has no
    /// SQLCipher (the key pragma would be ignored and the file would be plaintext) and a key that does not open the
    /// file, without changing anything and without putting the key in the message.
    pub(super) fn unlock(conn: &Connection, key: &Key) -> Result<()> {
        conn.pragma_update(None, "key", key.sqlcipher_pragma().as_str())
            .map_err(be)?;
        let version: String = conn
            .query_row("PRAGMA cipher_version", [], |r| r.get(0))
            .unwrap_or_default();
        if version.is_empty() {
            return Err(Error::Backend(
                "this build has no SQLCipher, so it cannot keep a store encrypted".into(),
            ));
        }
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))
            .map(|_| ())
            .map_err(|e| match &e {
                rusqlite::Error::SqliteFailure(f, _)
                    if matches!(f.code, rusqlite::ErrorCode::NotADatabase) =>
                {
                    Error::Backend(
                        "cannot open the store: wrong key, or the file is not an encrypted klatch store"
                            .into(),
                    )
                }
                _ => be(e),
            })
    }

    /// Re-encrypt the whole store under `new`, in place. The old key stops working. Other open connections keep their
    /// old key and fail until they reopen, so rotate when nothing else is using the store.
    pub fn rekey(&self, new: &Key) -> Result<()> {
        self.conn
            .pragma_update(None, "rekey", new.sqlcipher_pragma().as_str())
            .map_err(be)
    }

    /// The SQLCipher version this store runs on.
    pub fn cipher_version(&self) -> Result<String> {
        self.conn
            .query_row("PRAGMA cipher_version", [], |r| r.get(0))
            .map_err(be)
    }

    /// An unencrypted in-memory store, for tests: it is never written to disk.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory().map_err(be)?)
    }

    pub(super) fn init(conn: Connection) -> Result<Self> {
        // The version is re-checked inside the write lock so two processes opening an old
        // database at once cannot both migrate it.
        conn.execute_batch("BEGIN IMMEDIATE").map_err(be)?;
        let outcome = (|| -> Result<()> {
            let version = user_version(&conn)?;
            if version > SCHEMA_VERSION {
                return Err(Error::Backend(format!(
                    "this store was written by a newer klatch (schema {version}, this one knows {SCHEMA_VERSION}); upgrade klatch instead of opening it with an older one"
                )));
            }
            if version < SCHEMA_VERSION {
                if version == 0 && !has_table(&conn, "messages")? {
                    conn.execute_batch(SCHEMA).map_err(be)?;
                } else {
                    schema::migrate(&conn, version)?;
                }
                conn.pragma_update(None, "user_version", SCHEMA_VERSION)
                    .map_err(be)?;
            }
            Ok(())
        })();
        match outcome {
            Ok(()) => conn.execute_batch("COMMIT").map_err(be)?,
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(e);
            }
        }
        Ok(Self {
            conn,
            labels: std::cell::RefCell::default(),
            claim_record_errors: std::cell::Cell::default(),
            audit_flush_errors: std::cell::Cell::default(),
            deferred_audit: std::cell::RefCell::default(),
        })
    }
}
