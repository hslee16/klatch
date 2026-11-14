use crate::ids::new_id;
use crate::{ClaimRequest, HookEvent, NewAudit, NewMessage, NewRefusal, Release, Seat, Store};
use klatch_core::{
    AskRoom, AuditRecord, Claim, ClaimChanges, Error, HookRun, MemberInfo, Message, OpenAsk,
    Posted, RefusalRecord, ReleaseNote, ReleasedClaim, Result,
};
use klatch_keys::Key;
use rusqlite::{Connection, params};
use std::path::Path;
use std::time::Duration;

mod asks;
mod claims;
mod rows;
mod schema;
use rows::{MESSAGE_COLUMNS, attribution_text, message_from_row};
use schema::{SCHEMA, SCHEMA_VERSION, WAIT_WINDOW_SECS, has_table, user_version};

/// SQLite in WAL mode. Many readers, one writer at a time; a claim check-and-insert and a post are each one
/// `BEGIN IMMEDIATE` transaction so they are atomic across processes.
///
/// Data-model rules (DESIGN D6): every row carries a `tenant` (always `local` in v1) and a `member`
/// (until the `Identity` trait exists the member id is the member's name); message ids are globally unique
/// and separate from the dense per-room `seq`.
pub struct SqliteStore {
    conn: Connection,
}

fn be(e: rusqlite::Error) -> Error {
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
    /// The note `owner` left when it released `path`, if it was the latest release of that path (#272).
    fn note_of(&self, room: &str, owner: &str, path: &str) -> Result<Option<String>> {
        Ok(self
            .last_release(room, path)?
            .filter(|n| n.owner == owner)
            .map(|n| n.note))
    }

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

    fn try_open(path: &Path, key: &Key, busy: Duration) -> Result<Self> {
        let conn = Connection::open(path).map_err(be)?;
        Self::unlock(&conn, key)?;
        conn.busy_timeout(busy).map_err(be)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(be)?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(be)?;
        Self::init(conn)
    }

    /// Apply the key and check it: this must be the first thing done on a connection. Refuses a build that has no
    /// SQLCipher (the key pragma would be ignored and the file would be plaintext) and a key that does not open the
    /// file, without changing anything and without putting the key in the message.
    fn unlock(conn: &Connection, key: &Key) -> Result<()> {
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

    fn init(conn: Connection) -> Result<Self> {
        // The version is re-checked inside the write lock so two processes opening an old
        // database at once cannot both migrate it.
        conn.execute_batch("BEGIN IMMEDIATE").map_err(be)?;
        let outcome = (|| -> Result<()> {
            let version = user_version(&conn)?;
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
        Ok(Self { conn })
    }

    fn touch(&self, room: &str, me: &str, now: i64) -> Result<()> {
        self.exec(
            "INSERT INTO members(room,name,joined,last_seen) VALUES(?1,?2,?3,?3)
                 ON CONFLICT(room,name) DO UPDATE SET last_seen=?3",
            params![room, me, now],
        )?;
        Ok(())
    }

    /// Run one write statement in its own transaction.
    fn write(&self, sql: &str, params: impl rusqlite::Params) -> Result<()> {
        self.immediate(|| self.exec(sql, params).map(|_| ()))
    }

    /// Mark the member present in the room, then update its row: `sql` takes the room, the member and `value`.
    fn update_member(
        &self,
        (room, member, now): (&str, &str, i64),
        sql: &str,
        value: &dyn rusqlite::ToSql,
    ) -> Result<()> {
        self.immediate(|| {
            self.touch(room, member, now)?;
            self.exec(sql, params![room, member, value]).map(|_| ())
        })
    }

    /// Run one statement; how many rows it changed.
    fn exec(&self, sql: &str, params: impl rusqlite::Params) -> Result<usize> {
        self.conn.execute(sql, params).map_err(be)
    }

    /// The first row of a query, if there is one.
    fn optional<T>(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
        map: impl FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Option<T>> {
        match self.conn.query_row(sql, params, map) {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(be(e)),
        }
    }

    /// Every row of a query.
    fn rows<T>(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
        map: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Vec<T>> {
        let mut st = self.conn.prepare(sql).map_err(be)?;
        st.query_map(params, map)
            .map_err(be)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(be)
    }

    fn immediate<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        self.conn.execute_batch("BEGIN IMMEDIATE").map_err(be)?;
        match f() {
            Ok(v) => {
                self.conn.execute_batch("COMMIT").map_err(be)?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }
}

impl Store for SqliteStore {
    fn join(&self, room: &str, member: &str, now: i64) -> Result<()> {
        self.touch(room, member, now)
    }

    fn post_as(&self, message: &NewMessage, now: i64) -> Result<Posted> {
        let NewMessage {
            room,
            sender,
            attribution,
            label,
            body,
        } = *message;
        self.immediate(|| {
            let seq: i64 = self
                .conn
                .query_row(
                    "SELECT COALESCE(MAX(seq),0)+1 FROM messages WHERE room=?1",
                    params![room],
                    |r| r.get(0),
                )
                .map_err(be)?;
            let id = new_id((now.max(0) as u64) * 1000);
            self.exec(
                    "INSERT INTO messages(room,seq,mid,sender,member,body,ts,attribution,label) VALUES(?1,?2,?3,?4,?4,?5,?6,?7,?8)",
                    params![room, seq, id, sender, body, now, attribution_text(attribution), label],
                )?;
            self.touch(room, sender, now)?;
            Ok(Posted { id, seq })
        })
    }

    fn read(&self, at: Seat, advance: bool, now: i64) -> Result<Vec<Message>> {
        let Seat { room, member } = at;
        self.touch(room, member, now)?;
        let last: i64 = self
            .conn
            .query_row(
                "SELECT last_seq FROM members WHERE room=?1 AND name=?2",
                params![room, member],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let rows = self.rows(&format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE room=?1 AND seq>?2 AND sender<>?3 ORDER BY seq"), params![room, last, member], message_from_row)?;
        if advance {
            // Advance to the last message actually returned, never to the room's current maximum:
            // a message posted between the select above and this update would otherwise be skipped
            // for good. Own messages after the last returned one are simply filtered again next time.
            let cursor = rows.last().map_or(last, |m| m.seq.max(last));
            self.exec(
                "UPDATE members SET last_seq=?3, last_seen=?4 WHERE room=?1 AND name=?2",
                params![room, member, cursor, now],
            )?;
        }
        Ok(rows)
    }

    fn note_session(&self, session_id: &str, member: &str, now: i64) -> Result<()> {
        self.write(
            "INSERT INTO sessions(session_id,member,ts) VALUES(?1,?2,?3)
             ON CONFLICT(session_id) DO UPDATE SET member=?2, ts=?3",
            params![session_id, member, now],
        )
    }

    fn session_of_member(&self, member: &str) -> Result<Option<String>> {
        self.optional(
            "SELECT session_id FROM sessions WHERE member=?1 ORDER BY ts DESC LIMIT 1",
            params![member],
            |r| r.get(0),
        )
    }

    fn member_of_session(&self, session_id: &str) -> Result<Option<String>> {
        self.optional(
            "SELECT member FROM sessions WHERE session_id=?1",
            params![session_id],
            |r| r.get(0),
        )
    }

    fn prune_members(&self, older_than: i64, now: i64) -> Result<usize> {
        self.immediate(|| {
            let cutoff = now - older_than;
            self.exec("DELETE FROM sessions WHERE ts<?1", params![cutoff])?;
            let names: Vec<String> = self.rows(
                "SELECT DISTINCT name FROM members WHERE last_seen<?1",
                params![cutoff],
                |r| r.get(0),
            )?;
            let removed = self.exec("DELETE FROM members WHERE last_seen<?1", params![cutoff])?;
            for name in names {
                let still: i64 = self
                    .conn
                    .query_row(
                        "SELECT COUNT(*) FROM members WHERE name=?1",
                        params![name],
                        |r| r.get(0),
                    )
                    .map_err(be)?;
                if still > 0 {
                    continue;
                }
                for sql in [
                    "DELETE FROM claims_seen WHERE member=?1",
                    "DELETE FROM asks_reported WHERE member=?1",
                    "DELETE FROM asks_escalated WHERE member=?1",
                    "DELETE FROM hook_runs WHERE member=?1",
                    "DELETE FROM claim_waits WHERE member=?1",
                ] {
                    self.exec(sql, params![name])?;
                }
            }
            Ok(removed)
        })
    }

    fn member_scope(&self, member: &str) -> Result<Option<String>> {
        self.optional(
            "SELECT scope FROM member_scope WHERE member=?1",
            params![member],
            |r| r.get(0),
        )
    }

    fn set_member_scope(&self, member: &str, scope: &str, now: i64) -> Result<()> {
        self.write(
            "INSERT INTO member_scope(member,scope,ts) VALUES(?1,?2,?3)
             ON CONFLICT(member) DO UPDATE SET scope=?2, ts=?3",
            params![member, scope, now],
        )
    }

    fn set_status(&self, at: Seat, status: &str, now: i64) -> Result<()> {
        self.update_member(
            (at.room, at.member, now),
            "UPDATE members SET status=?3 WHERE room=?1 AND name=?2",
            &status,
        )
    }

    fn wait_poll(&self, room: &str, member: &str, now: i64) -> Result<Vec<Message>> {
        // A waiting member is present: every poll refreshes presence, as a read does (#237).
        self.touch(room, member, now)?;
        let (read, seen): (i64, i64) = self
            .conn
            .query_row(
                "SELECT COALESCE(last_seq,0), wait_seq FROM members WHERE room=?1 AND name=?2",
                params![room, member],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or((0, 0));
        let rows = self.rows(&format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE room=?1 AND seq>?2 AND sender<>?3 ORDER BY seq"), params![room, read.max(seen), member], message_from_row)?;
        Ok(rows)
    }

    fn wait_seen(&self, at: Seat, seq: i64, now: i64) -> Result<()> {
        self.update_member(
            (at.room, at.member, now),
            "UPDATE members SET wait_seq=MAX(wait_seq,?3) WHERE room=?1 AND name=?2",
            &seq,
        )
    }

    fn wake_poll(&self, room: &str, member: &str, now: i64) -> Result<Vec<Message>> {
        self.touch(room, member, now)?;
        let (read, seen): (i64, i64) = self
            .conn
            .query_row(
                "SELECT COALESCE(last_seq,0), wake_seq FROM members WHERE room=?1 AND name=?2",
                params![room, member],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or((0, 0));
        self.rows(&format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE room=?1 AND seq>?2 AND sender<>?3 ORDER BY seq"), params![room, read.max(seen), member], message_from_row)
    }

    fn wake_seen(&self, at: Seat, seq: i64, now: i64) -> Result<()> {
        self.update_member(
            (at.room, at.member, now),
            "UPDATE members SET wake_seq=MAX(wake_seq,?3) WHERE room=?1 AND name=?2",
            &seq,
        )
    }

    fn rooms_with_prefix(&self, prefix: &str) -> Result<Vec<AskRoom>> {
        self.rows(
            "SELECT m.room, COUNT(DISTINCT m.name), MAX(m.joined),
                        COALESCE((SELECT MAX(x.ts) FROM messages x WHERE x.room=m.room), 0)
                 FROM members m WHERE substr(m.room,1,?2)=?1 GROUP BY m.room ORDER BY m.room",
            params![prefix, prefix.chars().count() as i64],
            |r| {
                let seen: i64 = r.get(2)?;
                let said: i64 = r.get(3)?;
                Ok(AskRoom {
                    name: r.get(0)?,
                    members: r.get::<_, i64>(1)? as usize,
                    last_activity: seen.max(said),
                })
            },
        )
    }

    fn lapsed_asks(&self, room: &str, member: &str, now: i64) -> Result<Vec<(OpenAsk, String)>> {
        self.immediate(|| {
            let mut out = Vec::new();
            for a in self.open_asks(room, now)? {
                if a.sender != member || a.claimed {
                    continue;
                }
                let Some((owner, expires)) = self
                    .ask_lease(room, a.seq)?
                    .filter(|(_, expires)| *expires <= now)
                else {
                    continue;
                };
                if self.report_lapse_once((room, member), a.seq, expires)? {
                    out.push((a, owner));
                }
            }
            Ok(out)
        })
    }

    fn stale_asks(&self, at: Seat, now: i64, after: i64) -> Result<Vec<OpenAsk>> {
        let Seat { room, member } = at;
        self.immediate(|| {
            let stale = self
                .open_asks(room, now)?
                .into_iter()
                .filter(|a| asks::is_stale(a, member, now, after))
                .collect();
            self.report_once("asks_reported", (room, member), stale)
        })
    }

    fn escalated_asks(&self, room: &str, member: &str, now: i64) -> Result<Vec<OpenAsk>> {
        self.immediate(|| {
            let escalated = self
                .open_asks(room, now)?
                .into_iter()
                .filter(|a| a.sender == member && a.no_capable_member())
                .collect();
            self.report_once("asks_escalated", (room, member), escalated)
        })
    }

    fn record_hook_run(&self, run: &HookEvent, now: i64) -> Result<()> {
        let HookEvent {
            member,
            event,
            bytes,
        } = *run;
        self.write(
            "INSERT INTO hook_runs(member,event,last_ts,runs,last_bytes) VALUES(?1,?2,?3,1,?4)
             ON CONFLICT(member,event) DO UPDATE SET last_ts=?3, runs=runs+1, last_bytes=?4",
            params![member, event, now, bytes],
        )
    }

    fn hook_runs(&self, member: &str) -> Result<Vec<HookRun>> {
        self.rows(
            "SELECT event,last_ts,runs,last_bytes FROM hook_runs WHERE member=?1 ORDER BY event",
            params![member],
            |r| {
                Ok(HookRun {
                    event: r.get(0)?,
                    last_ts: r.get(1)?,
                    runs: r.get(2)?,
                    last_bytes: r.get(3)?,
                })
            },
        )
    }

    fn hook_read(&self, room: &str, member: &str, now: i64) -> Result<Vec<Message>> {
        self.immediate(|| {
            self.touch(room, member, now)?;
            let (read, shown): (i64, i64) = self
                .conn
                .query_row(
                    "SELECT COALESCE(last_seq,0), hook_seq FROM members WHERE room=?1 AND name=?2",
                    params![room, member],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(be)?;
            let from = read.max(shown);
            let rows = self.rows(&format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE room=?1 AND seq>?2 AND sender<>?3 ORDER BY seq"), params![room, from, member], message_from_row)?;
            if let Some(last) = rows.last() {
                self.exec(
                        "UPDATE members SET hook_seq=?3 WHERE room=?1 AND name=?2",
                        params![room, member, last.seq],
                    )?;
            }
            Ok(rows)
        })
    }

    fn members(&self, room: &str) -> Result<Vec<MemberInfo>> {
        self.rows(
            "SELECT m.name,m.last_seen,m.status,NULLIF(p.parent,'') FROM members m
             LEFT JOIN member_parent p ON p.member=m.name WHERE m.room=?1 ORDER BY m.name",
            params![room],
            |r| {
                Ok(MemberInfo {
                    name: r.get(0)?,
                    last_seen: r.get(1)?,
                    status: r.get(2)?,
                    parent: r.get(3)?,
                })
            },
        )
    }

    fn claim(&self, request: &ClaimRequest, now: i64) -> Result<()> {
        self.take_claim(request, now)
    }

    fn release(&self, room: &str, owner: &str, path: &str) -> Result<()> {
        if let Some(seq) = path
            .strip_prefix("ask:")
            .and_then(|n| n.parse::<i64>().ok())
        {
            // A release on purpose is not a lapse.
            self.exec(
                "DELETE FROM ask_leases WHERE room=?1 AND seq=?2 AND owner=?3",
                params![room, seq, owner],
            )?;
        }
        let n = self.exec(
            "DELETE FROM claims WHERE room=?1 AND path=?2 AND owner=?3",
            params![room, path, owner],
        )?;
        if n == 0 {
            return Err(Error::NotFound(format!("no claim by {owner} on {path}")));
        }
        Ok(())
    }

    fn release_with(&self, release: &Release, now: i64) -> Result<()> {
        let Release {
            room,
            owner,
            path,
            note,
        } = *release;
        self.immediate(|| {
            self.release(room, owner, path)?;
            if note.is_empty() {
                self.exec(
                    "DELETE FROM claim_notes WHERE room=?1 AND path=?2",
                    params![room, path],
                )?;
            } else {
                self.exec(
                    "INSERT INTO claim_notes(room,path,owner,note,ts) VALUES(?1,?2,?3,?4,?5)
                         ON CONFLICT(room,path) DO UPDATE SET owner=?3, note=?4, ts=?5",
                    params![room, path, owner, note, now],
                )?;
            }
            Ok(())
        })
    }

    fn last_release(&self, room: &str, path: &str) -> Result<Option<ReleaseNote>> {
        self.optional(
            "SELECT owner,note,ts FROM claim_notes WHERE room=?1 AND path=?2",
            params![room, path],
            |r| {
                Ok(ReleaseNote {
                    owner: r.get(0)?,
                    note: r.get(1)?,
                    ts: r.get(2)?,
                })
            },
        )
    }

    fn set_room_state(&self, room: &str, state: Option<&str>, now: i64) -> Result<()> {
        match state {
            Some(st) => self.conn.execute(
                "INSERT INTO room_state(room,state,ts) VALUES(?1,?2,?3)
                 ON CONFLICT(room) DO UPDATE SET state=?2, ts=?3",
                params![room, st, now],
            ),
            None => self
                .conn
                .execute("DELETE FROM room_state WHERE room=?1", params![room]),
        }
        .map(|_| ())
        .map_err(be)
    }

    fn room_state(&self, room: &str) -> Result<Option<String>> {
        self.optional(
            "SELECT state FROM room_state WHERE room=?1",
            params![room],
            |r| r.get(0),
        )
    }

    fn note_parent(&self, member: &str, parent: &str, now: i64) -> Result<()> {
        self.write(
            "INSERT OR IGNORE INTO member_parent(member,parent,ts) VALUES(?1,?2,?3)",
            params![member, parent, now],
        )
    }

    fn parent_of(&self, member: &str) -> Result<Option<String>> {
        self.optional(
            "SELECT parent FROM member_parent WHERE member=?1",
            params![member],
            |r| r.get(0),
        )
    }

    fn set_muted(&self, at: Seat, muted: bool, now: i64) -> Result<()> {
        self.update_member(
            (at.room, at.member, now),
            "UPDATE members SET muted=?3 WHERE room=?1 AND name=?2",
            &i64::from(muted),
        )
    }

    fn is_muted(&self, at: Seat) -> Result<bool> {
        let muted = self.optional(
            "SELECT muted FROM members WHERE room=?1 AND name=?2",
            params![at.room, at.member],
            |r| r.get::<_, i64>(0),
        )?;
        Ok(muted.is_some_and(|m| m != 0))
    }

    fn block_member(&self, room: &str, member: &str, now: i64) -> Result<()> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO member_blocks(room,member,ts) VALUES(?1,?2,?3)",
                params![room, member, now],
            )
            .map(|_| ())
            .map_err(be)
    }

    fn unblock_member(&self, room: &str, member: &str) -> Result<bool> {
        let n = self.exec(
            "DELETE FROM member_blocks WHERE room=?1 AND member=?2",
            params![room, member],
        )?;
        Ok(n > 0)
    }

    fn is_blocked(&self, room: &str, member: &str) -> Result<bool> {
        let n: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM member_blocks WHERE member=?2 AND (room=?1 OR room='*')",
                params![room, member],
                |r| r.get(0),
            )
            .map_err(be)?;
        Ok(n > 0)
    }

    fn remove_member(&self, room: &str, member: &str, now: i64) -> Result<usize> {
        self.immediate(|| {
            let released: usize = self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM claims WHERE room=?1 AND owner=?2 AND expires>?3",
                    params![room, member, now],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(be)? as usize;
            for sql in [
                "DELETE FROM claims WHERE room=?1 AND owner=?2",
                "DELETE FROM members WHERE room=?1 AND name=?2",
            ] {
                self.exec(sql, params![room, member])?;
            }
            Ok(released)
        })
    }

    fn record_audit(&self, entry: &NewAudit, now: i64) -> Result<()> {
        let NewAudit {
            action,
            target,
            detail,
        } = *entry;
        self.exec(
            "INSERT INTO admin_audit(ts,action,target,detail) VALUES(?1,?2,?3,?4)",
            params![now, action, target, detail],
        )
        .map(|_| ())
    }

    fn audit_log(&self, limit: usize) -> Result<Vec<AuditRecord>> {
        self.rows(
            "SELECT ts,action,target,detail FROM admin_audit ORDER BY id DESC LIMIT ?1",
            params![limit as i64],
            |r| {
                Ok(AuditRecord {
                    ts: r.get(0)?,
                    action: r.get(1)?,
                    target: r.get(2)?,
                    detail: r.get(3)?,
                })
            },
        )
    }

    fn policy_get(&self, key: &str) -> Result<Option<String>> {
        self.optional("SELECT value FROM policy WHERE key=?1", params![key], |r| {
            r.get(0)
        })
    }

    fn policy_set(&self, key: &str, value: Option<&str>, now: i64) -> Result<()> {
        match value {
            Some(v) => self.write(
                "INSERT INTO policy(key,value,ts) VALUES(?1,?2,?3)
                 ON CONFLICT(key) DO UPDATE SET value=?2, ts=?3",
                params![key, v, now],
            ),
            None => self.write("DELETE FROM policy WHERE key=?1", params![key]),
        }
    }

    fn policy_list(&self, prefix: &str) -> Result<Vec<(String, String)>> {
        self.rows(
            "SELECT key,value FROM policy WHERE substr(key,1,length(?1))=?1 ORDER BY key",
            params![prefix],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    }

    fn record_refusal(&self, entry: &NewRefusal, now: i64) -> Result<()> {
        let NewRefusal {
            member,
            room,
            operation,
            rule,
        } = *entry;
        self.write(
            "INSERT INTO refusals(ts,member,room,operation,rule) VALUES(?1,?2,?3,?4,?5)",
            params![now, member, room, operation, rule],
        )
    }

    fn refusals(&self, limit: usize) -> Result<Vec<RefusalRecord>> {
        self.rows(
            "SELECT ts,member,room,operation,rule FROM refusals ORDER BY id DESC LIMIT ?1",
            params![limit as i64],
            |r| {
                Ok(RefusalRecord {
                    ts: r.get(0)?,
                    member: r.get(1)?,
                    room: r.get(2)?,
                    operation: r.get(3)?,
                    rule: r.get(4)?,
                })
            },
        )
    }

    fn refusal_rules_since(&self, member: &str, since: i64) -> Result<Vec<String>> {
        self.rows(
            "SELECT rule FROM refusals WHERE member=?1 AND ts>=?2 ORDER BY id",
            params![member, since],
            |r| r.get(0),
        )
    }

    fn release_member_claims(&self, member: &str, now: i64) -> Result<usize> {
        self.immediate(|| {
            let held: Vec<(String, String, i64)> = self.rows("SELECT room,path,expires FROM claims WHERE owner=?1", params![member], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            let mut released = 0;
            for (room, path, expires) in held {
                if expires > now {
                    released += 1;
                    if let Some(seq) = path
                        .strip_prefix("ask:")
                        .and_then(|n| n.parse::<i64>().ok())
                    {
                        // ends the lease now, so the asker is told it lapsed
                        self.exec(
                                "UPDATE ask_leases SET expires=?4 WHERE room=?1 AND seq=?2 AND owner=?3 AND expires>?4",
                                params![room, seq, member, now],
                            )?;
                    }
                }
                self.exec(
                        "DELETE FROM claims WHERE room=?1 AND path=?2 AND owner=?3",
                        params![room, path, member],
                    )?;
            }
            Ok(released)
        })
    }

    fn claims(&self, room: &str, now: i64) -> Result<Vec<Claim>> {
        self.rows("SELECT path,owner,reason,expires,scope FROM claims WHERE room=?1 AND expires>?2 ORDER BY path", params![room, now], |r| {
            Ok(Claim {
                path: r.get(0)?,
                owner: r.get(1)?,
                reason: r.get(2)?,
                expires: r.get(3)?,
                scope: r.get(4)?,
            })
        })
    }

    fn claim_changes(&self, room: &str, member: &str, now: i64) -> Result<ClaimChanges> {
        self.immediate(|| {
            let current: Vec<Claim> = self
                .claims(room, now)?
                .into_iter()
                .filter(|c| c.owner != member)
                .collect();
            let mut st = self
                .conn
                .prepare("SELECT owner,path,expires FROM claims_seen WHERE room=?1 AND member=?2")
                .map_err(be)?;
            let seen: Vec<(String, String, i64)> = st
                .query_map(params![room, member], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .map_err(be)?
                .collect::<std::result::Result<_, _>>()
                .map_err(be)?;
            let added: Vec<Claim> = current
                .iter()
                .filter(|c| !seen.iter().any(|(o, p, e)| *o == c.owner && *p == c.path && *e == c.expires))
                .cloned()
                .collect();
            let mut released = Vec::new();
            for (o, p, _) in seen
                .iter()
                .filter(|(o, p, _)| !current.iter().any(|c| c.owner == *o && c.path == *p))
            {
                released.push(ReleasedClaim {
                    owner: o.clone(),
                    path: p.clone(),
                    note: self.note_of(room, o, p)?,
                });
            }
            if added.is_empty() && released.is_empty() {
                return Ok(ClaimChanges::default());
            }
            self.exec(
                    "DELETE FROM claims_seen WHERE room=?1 AND member=?2",
                    params![room, member],
                )?;
            for c in &current {
                self.exec(
                        "INSERT INTO claims_seen(room,member,owner,path,expires) VALUES(?1,?2,?3,?4,?5)",
                        params![room, member, c.owner, c.path, c.expires],
                    )?;
            }
            Ok(ClaimChanges { added, released })
        })
    }

    fn rooms_of(&self, member: &str) -> Result<Vec<String>> {
        self.rows(
            "SELECT room FROM members WHERE name=?1 ORDER BY room",
            params![member],
            |r| r.get(0),
        )
    }

    fn tail(&self, room: &str, after_seq: i64, limit: usize) -> Result<Vec<Message>> {
        self.rows(&format!(
                "SELECT {MESSAGE_COLUMNS} FROM messages WHERE room=?1 AND seq>?2 ORDER BY seq LIMIT ?3"
            ), params![room, after_seq, limit as i64], message_from_row)
    }

    fn head(&self, room: &str) -> Result<i64> {
        self.conn
            .query_row(
                "SELECT COALESCE(MAX(seq),0) FROM messages WHERE room=?1",
                params![room],
                |r| r.get(0),
            )
            .map_err(be)
    }

    fn advance_cursor(&self, at: Seat, seq: i64, now: i64) -> Result<()> {
        let Seat { room, member } = at;
        // One statement, so it is atomic: forward only, and clamped to the room's head so an
        // acknowledgement can never skip a message that has not arrived yet.
        self.exec(
                "INSERT INTO members(room,name,joined,last_seen,last_seq)
                 VALUES(?1,?2,?4,?4,MAX(0,MIN(?3,(SELECT COALESCE(MAX(seq),0) FROM messages WHERE room=?1))))
                 ON CONFLICT(room,name) DO UPDATE SET
                   last_seq=MAX(last_seq,MIN(?3,(SELECT COALESCE(MAX(seq),0) FROM messages WHERE room=?1))),
                   last_seen=?4",
                params![room, member, seq, now],
            )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
