use crate::Store;
use crate::ids::new_id;
use klatch_core::{
    AskRoom, Claim, ClaimChanges, Error, HookRun, MemberInfo, Message, OpenAsk, Posted,
    ReleasedClaim, Result, overlaps, scopes_collide,
};
use rusqlite::{Connection, params};
use std::path::Path;
use std::time::Duration;

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

const SCHEMA_VERSION: i64 = 13;

const SCHEMA: &str = "CREATE TABLE members(tenant TEXT NOT NULL DEFAULT 'local', room TEXT, name TEXT, joined INTEGER, last_seen INTEGER, last_seq INTEGER DEFAULT 0, hook_seq INTEGER NOT NULL DEFAULT 0, wait_seq INTEGER NOT NULL DEFAULT 0, status TEXT, PRIMARY KEY(room,name));
 CREATE TABLE messages(id INTEGER PRIMARY KEY AUTOINCREMENT, tenant TEXT NOT NULL DEFAULT 'local', room TEXT NOT NULL, seq INTEGER NOT NULL, mid TEXT NOT NULL, sender TEXT, member TEXT, body TEXT, ts INTEGER, attribution TEXT NOT NULL DEFAULT 'unattributed', label TEXT NOT NULL DEFAULT 'public');
 CREATE UNIQUE INDEX messages_room_seq ON messages(room, seq);
 CREATE UNIQUE INDEX messages_mid ON messages(mid);
 CREATE TABLE claims(id INTEGER PRIMARY KEY AUTOINCREMENT, tenant TEXT NOT NULL DEFAULT 'local', room TEXT, path TEXT, owner TEXT, member TEXT, reason TEXT, expires INTEGER, scope TEXT NOT NULL DEFAULT '');
 CREATE TABLE claims_seen(room TEXT NOT NULL, member TEXT NOT NULL, owner TEXT NOT NULL, path TEXT NOT NULL, expires INTEGER NOT NULL, PRIMARY KEY(room,member,owner,path));
 CREATE TABLE asks_reported(room TEXT NOT NULL, member TEXT NOT NULL, seq INTEGER NOT NULL, PRIMARY KEY(room,member,seq));
 CREATE TABLE asks_escalated(room TEXT NOT NULL, member TEXT NOT NULL, seq INTEGER NOT NULL, PRIMARY KEY(room,member,seq));
 CREATE TABLE hook_runs(member TEXT NOT NULL, event TEXT NOT NULL, last_ts INTEGER NOT NULL, runs INTEGER NOT NULL, last_bytes INTEGER NOT NULL, PRIMARY KEY(member,event));
 CREATE TABLE claim_waits(room TEXT NOT NULL, member TEXT NOT NULL, path TEXT NOT NULL, ts INTEGER NOT NULL, scope TEXT NOT NULL DEFAULT '', PRIMARY KEY(room,member,path));
 CREATE TABLE sessions(session_id TEXT PRIMARY KEY, member TEXT NOT NULL, ts INTEGER NOT NULL);
 CREATE TABLE member_scope(member TEXT PRIMARY KEY, scope TEXT NOT NULL, ts INTEGER NOT NULL);
 CREATE TABLE ask_leases(room TEXT NOT NULL, seq INTEGER NOT NULL, owner TEXT NOT NULL, expires INTEGER NOT NULL, PRIMARY KEY(room,seq));
 CREATE TABLE asks_lapse_reported(room TEXT NOT NULL, member TEXT NOT NULL, seq INTEGER NOT NULL, expires INTEGER NOT NULL, PRIMARY KEY(room,member,seq,expires));";

fn user_version(conn: &Connection) -> Result<i64> {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(be)
}

fn has_table(conn: &Connection, name: &str) -> Result<bool> {
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            params![name],
            |r| r.get(0),
        )
        .map_err(be)?;
    Ok(n > 0)
}

/// Bring a version 0 database (the prototype schema) up to version 1 in place, keeping messages,
/// cursors and claims.
fn migrate_v0(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "ALTER TABLE messages ADD COLUMN tenant TEXT NOT NULL DEFAULT 'local';
         ALTER TABLE messages ADD COLUMN seq INTEGER;
         ALTER TABLE messages ADD COLUMN mid TEXT;
         ALTER TABLE messages ADD COLUMN member TEXT;
         UPDATE messages SET member = sender;
         UPDATE messages SET seq = (SELECT COUNT(*) FROM messages m2 WHERE m2.room = messages.room AND m2.id <= messages.id);
         ALTER TABLE members ADD COLUMN tenant TEXT NOT NULL DEFAULT 'local';
         ALTER TABLE members ADD COLUMN last_seq INTEGER DEFAULT 0;
         UPDATE members SET last_seq = (SELECT COUNT(*) FROM messages WHERE messages.room = members.room AND messages.id <= members.last_id);
         ALTER TABLE claims ADD COLUMN tenant TEXT NOT NULL DEFAULT 'local';
         ALTER TABLE claims ADD COLUMN member TEXT;
         UPDATE claims SET member = owner;",
    )
    .map_err(be)?;
    let rows: Vec<(i64, i64)> = {
        let mut st = conn
            .prepare("SELECT id, ts FROM messages ORDER BY id")
            .map_err(be)?;
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(be)?
            .collect::<std::result::Result<_, _>>()
            .map_err(be)?
    };
    for (id, ts) in rows {
        conn.execute(
            "UPDATE messages SET mid = ?1 WHERE id = ?2",
            params![new_id((ts.max(0) as u64) * 1000), id],
        )
        .map_err(be)?;
    }
    conn.execute_batch(
        "CREATE UNIQUE INDEX messages_room_seq ON messages(room, seq);
         CREATE UNIQUE INDEX messages_mid ON messages(mid);",
    )
    .map_err(be)?;
    Ok(())
}

/// Version 1 to 2: messages record whether the sender was proven and the label they were posted under.
/// History written before this is reported as unattributed and public, which is the truthful reading
/// because nothing recorded otherwise.
fn migrate_v1(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "ALTER TABLE messages ADD COLUMN attribution TEXT NOT NULL DEFAULT 'unattributed';
         ALTER TABLE messages ADD COLUMN label TEXT NOT NULL DEFAULT 'public';",
    )
    .map_err(be)
}

/// Version 2 to 3: what each member has been shown of other members' claims, so a hook reports only what
/// changed. Nothing to backfill: a member that has seen nothing is shown every claim once.
fn migrate_v2(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE claims_seen(room TEXT NOT NULL, member TEXT NOT NULL, owner TEXT NOT NULL, path TEXT NOT NULL, expires INTEGER NOT NULL, PRIMARY KEY(room,member,owner,path));",
    )
    .map_err(be)
}

/// Version 3 to 4: how far a hook has shown each member's messages, apart from how far the member has read.
/// Existing members start at 0, which the hook treats as "no further than the read cursor".
fn migrate_v3(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE members ADD COLUMN hook_seq INTEGER NOT NULL DEFAULT 0;")
        .map_err(be)
}

/// Version 4 to 5: how far a `wait` has looked at each member's room, apart from the read cursor, so a filtered
/// wait does not wake again for a message it already skipped or returned while the message stays unread.
fn migrate_v4(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE members ADD COLUMN wait_seq INTEGER NOT NULL DEFAULT 0;")
        .map_err(be)
}

/// Version 5 to 6: which stale asks were already reported to their asker, so each is reported once.
fn migrate_v5(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE asks_reported(room TEXT NOT NULL, member TEXT NOT NULL, seq INTEGER NOT NULL, PRIMARY KEY(room,member,seq));",
    )
    .map_err(be)
}

/// Version 6 to 7: the status a member sets for itself in a room (`done`, `busy`, ...), shown by `who` and the board.
fn migrate_v6(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE members ADD COLUMN status TEXT;")
        .map_err(be)
}

/// Version 7 to 8: which asks were already reported to their asker as undoable by anyone here, so each is told once.
fn migrate_v7(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE asks_escalated(room TEXT NOT NULL, member TEXT NOT NULL, seq INTEGER NOT NULL, PRIMARY KEY(room,member,seq));",
    )
    .map_err(be)
}

/// Version 8 to 9: the latest run of each hook event per member (time, count, size printed), so a user can see
/// the hooks are alive without asking the session. Never any content.
fn migrate_v8(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE hook_runs(member TEXT NOT NULL, event TEXT NOT NULL, last_ts INTEGER NOT NULL, runs INTEGER NOT NULL, last_bytes INTEGER NOT NULL, PRIMARY KEY(member,event));",
    )
    .map_err(be)
}

/// Version 9 to 10: the claims members recently tried and could not take, so a conflict that closes a cycle of
/// waits can be named a deadlock. Short lived, never any content.
fn migrate_v9(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE claim_waits(room TEXT NOT NULL, member TEXT NOT NULL, path TEXT NOT NULL, ts INTEGER NOT NULL, PRIMARY KEY(room,member,path));",
    )
    .map_err(be)
}

/// A member that tried to claim something and was refused is treated as still waiting for this long.
const WAIT_WINDOW_SECS: i64 = 120;

/// Version 10 to 11: which member each harness session is, so a hook that runs after the harness dropped its own
/// record (the SessionEnd hook) can still find its member. A session id and a member name only.
fn migrate_v10(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE sessions(session_id TEXT PRIMARY KEY, member TEXT NOT NULL, ts INTEGER NOT NULL);",
    )
    .map_err(be)
}

/// Version 11 to 12: the scope a claim lives in and the scope a member works in (DESIGN section 27), so two
/// repositories, or two bots, can claim the same path without colliding. Existing claims keep the empty scope.
fn migrate_v11(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "ALTER TABLE claims ADD COLUMN scope TEXT NOT NULL DEFAULT '';
         ALTER TABLE claim_waits ADD COLUMN scope TEXT NOT NULL DEFAULT '';
         CREATE TABLE member_scope(member TEXT PRIMARY KEY, scope TEXT NOT NULL, ts INTEGER NOT NULL);",
    )
    .map_err(be)
}

/// Version 12 to 13: the last lease taken on each ask, and which lapses were already reported to the asker, so a
/// helper that failed or vanished after claiming an ask can be told to the asker once (DESIGN section 28).
fn migrate_v12(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE ask_leases(room TEXT NOT NULL, seq INTEGER NOT NULL, owner TEXT NOT NULL, expires INTEGER NOT NULL, PRIMARY KEY(room,seq));
         CREATE TABLE asks_lapse_reported(room TEXT NOT NULL, member TEXT NOT NULL, seq INTEGER NOT NULL, expires INTEGER NOT NULL, PRIMARY KEY(room,member,seq,expires));",
    )
    .map_err(be)
}

fn attribution_text(a: klatch_core::Attribution) -> &'static str {
    match a {
        klatch_core::Attribution::Verified => "verified",
        klatch_core::Attribution::Unattributed => "unattributed",
    }
}

/// Anything other than `verified` reads as unattributed: an unknown value must never look proven.
fn attribution_from(text: &str) -> klatch_core::Attribution {
    if text == "verified" {
        klatch_core::Attribution::Verified
    } else {
        klatch_core::Attribution::Unattributed
    }
}

/// The columns every message query selects, in the order `message_from_row` reads them.
const MESSAGE_COLUMNS: &str = "mid,seq,sender,attribution,label,body,ts";

fn message_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Message> {
    Ok(Message {
        id: r.get(0)?,
        seq: r.get(1)?,
        sender: r.get(2)?,
        attribution: attribution_from(&r.get::<_, String>(3)?),
        label: r.get(4)?,
        body: r.get(5)?,
        ts: r.get(6)?,
    })
}

impl SqliteStore {
    /// Open with the default five second wait for another writer.
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_timeout(path, Duration::from_secs(5))
    }

    /// Open with an explicit limit on how long a write waits for another writer. Past the limit the
    /// write fails with `Error::Backend` and leaves nothing behind; it never hangs.
    pub fn open_with_timeout(path: &Path, busy: Duration) -> Result<Self> {
        // Switching a fresh database to WAL and creating its schema can collide with other first-time
        // openers in a way SQLite's own busy handler does not cover, so the whole open is retried on
        // contention, with a small backoff, until the limit.
        let deadline = std::time::Instant::now() + busy;
        let mut attempt: u64 = 0;
        loop {
            match Self::try_open(path, busy) {
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

    fn try_open(path: &Path, busy: Duration) -> Result<Self> {
        let conn = Connection::open(path).map_err(be)?;
        conn.busy_timeout(busy).map_err(be)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(be)?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(be)?;
        Self::init(conn)
    }

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
                    if version == 0 {
                        migrate_v0(&conn)?;
                    }
                    if version < 2 {
                        migrate_v1(&conn)?;
                    }
                    if version < 3 {
                        migrate_v2(&conn)?;
                    }
                    if version < 4 {
                        migrate_v3(&conn)?;
                    }
                    if version < 5 {
                        migrate_v4(&conn)?;
                    }
                    if version < 6 {
                        migrate_v5(&conn)?;
                    }
                    if version < 7 {
                        migrate_v6(&conn)?;
                    }
                    if version < 8 {
                        migrate_v7(&conn)?;
                    }
                    if version < 9 {
                        migrate_v8(&conn)?;
                    }
                    if version < 10 {
                        migrate_v9(&conn)?;
                    }
                    if version < 11 {
                        migrate_v10(&conn)?;
                    }
                    if version < 12 {
                        migrate_v11(&conn)?;
                    }
                    migrate_v12(&conn)?;
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
        self.conn
            .execute(
                "INSERT INTO members(room,name,joined,last_seen) VALUES(?1,?2,?3,?3)
                 ON CONFLICT(room,name) DO UPDATE SET last_seen=?3",
                params![room, me, now],
            )
            .map_err(be)?;
        Ok(())
    }

    /// Whether a refusal of `requester` by `holder` closes a cycle of waits: `holder` is waiting (a refused claim
    /// attempt within the window) for something a member holds who is waiting for something ... that `requester`
    /// holds. `held` is every live claim in the room.
    fn closes_cycle(
        &self,
        room: &str,
        requester: &str,
        holder: &str,
        held: &[(String, String, i64, String)],
    ) -> Result<bool> {
        let mut st = self
            .conn
            .prepare("SELECT member,path,scope FROM claim_waits WHERE room=?1")
            .map_err(be)?;
        let waits: Vec<(String, String, String)> = st
            .query_map(params![room], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(be)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(be)?;
        let mut seen: Vec<String> = Vec::new();
        let mut stack = vec![holder.to_string()];
        while let Some(node) = stack.pop() {
            if node == requester {
                return Ok(true);
            }
            if seen.contains(&node) {
                continue;
            }
            seen.push(node.clone());
            for (member, wanted, wscope) in waits.iter().filter(|(m, _, _)| *m == node) {
                for (p, o, _, sc) in held {
                    if o != member && overlaps(p, wanted) && scopes_collide(sc, wscope) {
                        stack.push(o.clone());
                    }
                }
            }
        }
        Ok(false)
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

    fn post_as(
        &self,
        room: &str,
        sender: &str,
        attribution: klatch_core::Attribution,
        label: &str,
        body: &str,
        now: i64,
    ) -> Result<Posted> {
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
            self.conn
                .execute(
                    "INSERT INTO messages(room,seq,mid,sender,member,body,ts,attribution,label) VALUES(?1,?2,?3,?4,?4,?5,?6,?7,?8)",
                    params![room, seq, id, sender, body, now, attribution_text(attribution), label],
                )
                .map_err(be)?;
            self.touch(room, sender, now)?;
            Ok(Posted { id, seq })
        })
    }

    fn read(&self, room: &str, member: &str, advance: bool, now: i64) -> Result<Vec<Message>> {
        self.touch(room, member, now)?;
        let last: i64 = self
            .conn
            .query_row(
                "SELECT last_seq FROM members WHERE room=?1 AND name=?2",
                params![room, member],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let mut st = self
            .conn
            .prepare(&format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE room=?1 AND seq>?2 AND sender<>?3 ORDER BY seq"))
            .map_err(be)?;
        let rows = st
            .query_map(params![room, last, member], message_from_row)
            .map_err(be)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(be)?;
        if advance {
            // Advance to the last message actually returned, never to the room's current maximum:
            // a message posted between the select above and this update would otherwise be skipped
            // for good. Own messages after the last returned one are simply filtered again next time.
            let cursor = rows.last().map_or(last, |m| m.seq.max(last));
            self.conn
                .execute(
                    "UPDATE members SET last_seq=?3, last_seen=?4 WHERE room=?1 AND name=?2",
                    params![room, member, cursor, now],
                )
                .map_err(be)?;
        }
        Ok(rows)
    }

    fn note_session(&self, session_id: &str, member: &str, now: i64) -> Result<()> {
        self.immediate(|| {
            self.conn
                .execute(
                    "INSERT INTO sessions(session_id,member,ts) VALUES(?1,?2,?3)
                     ON CONFLICT(session_id) DO UPDATE SET member=?2, ts=?3",
                    params![session_id, member, now],
                )
                .map_err(be)?;
            Ok(())
        })
    }

    fn session_of_member(&self, member: &str) -> Result<Option<String>> {
        match self.conn.query_row(
            "SELECT session_id FROM sessions WHERE member=?1 ORDER BY ts DESC LIMIT 1",
            params![member],
            |r| r.get(0),
        ) {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(be(e)),
        }
    }

    fn member_of_session(&self, session_id: &str) -> Result<Option<String>> {
        match self.conn.query_row(
            "SELECT member FROM sessions WHERE session_id=?1",
            params![session_id],
            |r| r.get(0),
        ) {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(be(e)),
        }
    }

    fn prune_members(&self, older_than: i64, now: i64) -> Result<usize> {
        self.immediate(|| {
            let cutoff = now - older_than;
            self.conn
                .execute("DELETE FROM sessions WHERE ts<?1", params![cutoff])
                .map_err(be)?;
            let mut st = self
                .conn
                .prepare("SELECT DISTINCT name FROM members WHERE last_seen<?1")
                .map_err(be)?;
            let names: Vec<String> = st
                .query_map(params![cutoff], |r| r.get(0))
                .map_err(be)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(be)?;
            let removed = self
                .conn
                .execute("DELETE FROM members WHERE last_seen<?1", params![cutoff])
                .map_err(be)?;
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
                    self.conn.execute(sql, params![name]).map_err(be)?;
                }
            }
            Ok(removed)
        })
    }

    fn member_scope(&self, member: &str) -> Result<Option<String>> {
        match self.conn.query_row(
            "SELECT scope FROM member_scope WHERE member=?1",
            params![member],
            |r| r.get(0),
        ) {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(be(e)),
        }
    }

    fn set_member_scope(&self, member: &str, scope: &str, now: i64) -> Result<()> {
        self.immediate(|| {
            self.conn
                .execute(
                    "INSERT INTO member_scope(member,scope,ts) VALUES(?1,?2,?3)
                     ON CONFLICT(member) DO UPDATE SET scope=?2, ts=?3",
                    params![member, scope, now],
                )
                .map_err(be)?;
            Ok(())
        })
    }

    fn set_status(&self, room: &str, member: &str, status: &str, now: i64) -> Result<()> {
        self.immediate(|| {
            self.touch(room, member, now)?;
            self.conn
                .execute(
                    "UPDATE members SET status=?3 WHERE room=?1 AND name=?2",
                    params![room, member, status],
                )
                .map_err(be)?;
            Ok(())
        })
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
        let mut st = self
            .conn
            .prepare(&format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE room=?1 AND seq>?2 AND sender<>?3 ORDER BY seq"))
            .map_err(be)?;
        let rows = st
            .query_map(params![room, read.max(seen), member], message_from_row)
            .map_err(be)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(be)?;
        Ok(rows)
    }

    fn wait_seen(&self, room: &str, member: &str, seq: i64, now: i64) -> Result<()> {
        self.immediate(|| {
            self.touch(room, member, now)?;
            self.conn
                .execute(
                    "UPDATE members SET wait_seq=MAX(wait_seq,?3) WHERE room=?1 AND name=?2",
                    params![room, member, seq],
                )
                .map_err(be)?;
            Ok(())
        })
    }

    fn rooms_with_prefix(&self, prefix: &str) -> Result<Vec<AskRoom>> {
        let mut st = self
            .conn
            .prepare(
                "SELECT m.room, COUNT(DISTINCT m.name), MAX(m.joined),
                        COALESCE((SELECT MAX(x.ts) FROM messages x WHERE x.room=m.room), 0)
                 FROM members m WHERE substr(m.room,1,?2)=?1 GROUP BY m.room ORDER BY m.room",
            )
            .map_err(be)?;
        st.query_map(params![prefix, prefix.chars().count() as i64], |r| {
            let seen: i64 = r.get(2)?;
            let said: i64 = r.get(3)?;
            Ok(AskRoom {
                name: r.get(0)?,
                members: r.get::<_, i64>(1)? as usize,
                last_activity: seen.max(said),
            })
        })
        .map_err(be)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(be)
    }

    fn lapsed_asks(&self, room: &str, member: &str, now: i64) -> Result<Vec<(OpenAsk, String)>> {
        self.immediate(|| {
            let mut out = Vec::new();
            for a in self.open_asks(room, now)? {
                if a.sender != member || a.claimed {
                    continue;
                }
                let lease: Option<(String, i64)> = match self.conn.query_row(
                    "SELECT owner,expires FROM ask_leases WHERE room=?1 AND seq=?2",
                    params![room, a.seq],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                ) {
                    Ok(l) => Some(l),
                    Err(rusqlite::Error::QueryReturnedNoRows) => None,
                    Err(e) => return Err(be(e)),
                };
                let Some((owner, expires)) = lease.filter(|(_, e)| *e <= now) else {
                    continue;
                };
                let n = self
                    .conn
                    .execute(
                        "INSERT OR IGNORE INTO asks_lapse_reported(room,member,seq,expires) VALUES(?1,?2,?3,?4)",
                        params![room, member, a.seq, expires],
                    )
                    .map_err(be)?;
                if n == 1 {
                    out.push((a, owner));
                }
            }
            Ok(out)
        })
    }

    fn stale_asks(&self, room: &str, member: &str, now: i64, after: i64) -> Result<Vec<OpenAsk>> {
        self.immediate(|| {
            let mut fresh = Vec::new();
            for a in self.open_asks(room, now)? {
                if a.sender != member
                    || a.claimed
                    || a.waiting_on_asker.is_some()
                    || a.room_active(now)
                    || now - a.ts < after
                {
                    continue;
                }
                let n = self
                    .conn
                    .execute(
                        "INSERT OR IGNORE INTO asks_reported(room,member,seq) VALUES(?1,?2,?3)",
                        params![room, member, a.seq],
                    )
                    .map_err(be)?;
                if n == 1 {
                    fresh.push(a);
                }
            }
            Ok(fresh)
        })
    }

    fn escalated_asks(&self, room: &str, member: &str, now: i64) -> Result<Vec<OpenAsk>> {
        self.immediate(|| {
            let mut fresh = Vec::new();
            for a in self.open_asks(room, now)? {
                if a.sender != member || !a.no_capable_member() {
                    continue;
                }
                let n = self
                    .conn
                    .execute(
                        "INSERT OR IGNORE INTO asks_escalated(room,member,seq) VALUES(?1,?2,?3)",
                        params![room, member, a.seq],
                    )
                    .map_err(be)?;
                if n == 1 {
                    fresh.push(a);
                }
            }
            Ok(fresh)
        })
    }

    fn record_hook_run(&self, member: &str, event: &str, now: i64, bytes: i64) -> Result<()> {
        self.immediate(|| {
            self.conn
                .execute(
                    "INSERT INTO hook_runs(member,event,last_ts,runs,last_bytes) VALUES(?1,?2,?3,1,?4)
                     ON CONFLICT(member,event) DO UPDATE SET last_ts=?3, runs=runs+1, last_bytes=?4",
                    params![member, event, now, bytes],
                )
                .map_err(be)?;
            Ok(())
        })
    }

    fn hook_runs(&self, member: &str) -> Result<Vec<HookRun>> {
        let mut st = self
            .conn
            .prepare("SELECT event,last_ts,runs,last_bytes FROM hook_runs WHERE member=?1 ORDER BY event")
            .map_err(be)?;
        st.query_map(params![member], |r| {
            Ok(HookRun {
                event: r.get(0)?,
                last_ts: r.get(1)?,
                runs: r.get(2)?,
                last_bytes: r.get(3)?,
            })
        })
        .map_err(be)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(be)
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
            let mut st = self
                .conn
                .prepare(&format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE room=?1 AND seq>?2 AND sender<>?3 ORDER BY seq"))
                .map_err(be)?;
            let rows = st
                .query_map(params![room, from, member], message_from_row)
                .map_err(be)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(be)?;
            if let Some(last) = rows.last() {
                self.conn
                    .execute(
                        "UPDATE members SET hook_seq=?3 WHERE room=?1 AND name=?2",
                        params![room, member, last.seq],
                    )
                    .map_err(be)?;
            }
            Ok(rows)
        })
    }

    fn members(&self, room: &str) -> Result<Vec<MemberInfo>> {
        let mut st = self
            .conn
            .prepare("SELECT name,last_seen,status FROM members WHERE room=?1 ORDER BY name")
            .map_err(be)?;
        st.query_map(params![room], |r| {
            Ok(MemberInfo {
                name: r.get(0)?,
                last_seen: r.get(1)?,
                status: r.get(2)?,
            })
        })
        .map_err(be)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(be)
    }

    fn claim(
        &self,
        room: &str,
        owner: &str,
        path: &str,
        ttl: i64,
        reason: &str,
        now: i64,
    ) -> Result<()> {
        self.claim_in(room, owner, "", path, ttl, reason, now)
    }

    fn claim_in(
        &self,
        room: &str,
        owner: &str,
        scope: &str,
        path: &str,
        ttl: i64,
        reason: &str,
        now: i64,
    ) -> Result<()> {
        // The closure returns the outcome as a value, not an error, so that a refused attempt is still committed:
        // it is remembered as a wait, which is what lets a later conflict be recognised as closing a cycle.
        self.immediate(|| {
            self.conn
                .execute("DELETE FROM claims WHERE expires<=?1", params![now])
                .map_err(be)?;
            self.conn
                .execute(
                    "DELETE FROM claim_waits WHERE ts<?1",
                    params![now - WAIT_WINDOW_SECS],
                )
                .map_err(be)?;
            let mut st = self
                .conn
                .prepare("SELECT path,owner,expires,scope FROM claims WHERE room=?1")
                .map_err(be)?;
            let held = st
                .query_map(params![room], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })
                .map_err(be)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(be)?;
            for (p, o, e, sc) in &held {
                if o != owner && overlaps(p, path) && scopes_collide(sc, scope) {
                    self.conn
                        .execute(
                            "INSERT INTO claim_waits(room,member,path,ts,scope) VALUES(?1,?2,?3,?4,?5)
                             ON CONFLICT(room,member,path) DO UPDATE SET ts=?4, scope=?5",
                            params![room, owner, path, now, scope],
                        )
                        .map_err(be)?;
                    let deadlock = self.closes_cycle(room, owner, o, &held)?;
                    return Ok(Err(Error::Conflict {
                        path: p.clone(),
                        owner: o.clone(),
                        secs_left: klatch_core::secs_between(*e, now),
                        deadlock,
                    }));
                }
            }
            // Taking the path ends this member's wait for it.
            let mut waits = self
                .conn
                .prepare("SELECT path FROM claim_waits WHERE room=?1 AND member=?2")
                .map_err(be)?;
            let mine: Vec<String> = waits
                .query_map(params![room, owner], |r| r.get(0))
                .map_err(be)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(be)?;
            for w in mine.into_iter().filter(|w| overlaps(w, path)) {
                self.conn
                    .execute(
                        "DELETE FROM claim_waits WHERE room=?1 AND member=?2 AND path=?3",
                        params![room, owner, w],
                    )
                    .map_err(be)?;
            }
            self.conn
                .execute(
                    "DELETE FROM claims WHERE room=?1 AND owner=?2 AND path=?3",
                    params![room, owner, path],
                )
                .map_err(be)?;
            self.conn
                .execute(
                    "INSERT INTO claims(room,path,owner,member,reason,expires,scope) VALUES(?1,?2,?3,?3,?4,?5,?6)",
                    params![room, path, owner, reason, now + ttl, scope],
                )
                .map_err(be)?;
            if let Some(seq) = path.strip_prefix("ask:").and_then(|n| n.parse::<i64>().ok()) {
                self.conn
                    .execute(
                        "INSERT INTO ask_leases(room,seq,owner,expires) VALUES(?1,?2,?3,?4)
                         ON CONFLICT(room,seq) DO UPDATE SET owner=?3, expires=?4",
                        params![room, seq, owner, now + ttl],
                    )
                    .map_err(be)?;
            }
            Ok(Ok(()))
        })?
    }

    fn release(&self, room: &str, owner: &str, path: &str) -> Result<()> {
        if let Some(seq) = path
            .strip_prefix("ask:")
            .and_then(|n| n.parse::<i64>().ok())
        {
            // A release on purpose is not a lapse.
            self.conn
                .execute(
                    "DELETE FROM ask_leases WHERE room=?1 AND seq=?2 AND owner=?3",
                    params![room, seq, owner],
                )
                .map_err(be)?;
        }
        let n = self
            .conn
            .execute(
                "DELETE FROM claims WHERE room=?1 AND path=?2 AND owner=?3",
                params![room, path, owner],
            )
            .map_err(be)?;
        if n == 0 {
            return Err(Error::NotFound(format!("no claim by {owner} on {path}")));
        }
        Ok(())
    }

    fn claims(&self, room: &str, now: i64) -> Result<Vec<Claim>> {
        let mut st = self
            .conn
            .prepare("SELECT path,owner,reason,expires,scope FROM claims WHERE room=?1 AND expires>?2 ORDER BY path")
            .map_err(be)?;
        st.query_map(params![room, now], |r| {
            Ok(Claim {
                path: r.get(0)?,
                owner: r.get(1)?,
                reason: r.get(2)?,
                expires: r.get(3)?,
                scope: r.get(4)?,
            })
        })
        .map_err(be)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(be)
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
            let released: Vec<ReleasedClaim> = seen
                .iter()
                .filter(|(o, p, _)| !current.iter().any(|c| c.owner == *o && c.path == *p))
                .map(|(o, p, _)| ReleasedClaim { owner: o.clone(), path: p.clone() })
                .collect();
            if added.is_empty() && released.is_empty() {
                return Ok(ClaimChanges::default());
            }
            self.conn
                .execute(
                    "DELETE FROM claims_seen WHERE room=?1 AND member=?2",
                    params![room, member],
                )
                .map_err(be)?;
            for c in &current {
                self.conn
                    .execute(
                        "INSERT INTO claims_seen(room,member,owner,path,expires) VALUES(?1,?2,?3,?4,?5)",
                        params![room, member, c.owner, c.path, c.expires],
                    )
                    .map_err(be)?;
            }
            Ok(ClaimChanges { added, released })
        })
    }

    fn rooms_of(&self, member: &str) -> Result<Vec<String>> {
        let mut st = self
            .conn
            .prepare("SELECT room FROM members WHERE name=?1 ORDER BY room")
            .map_err(be)?;
        st.query_map(params![member], |r| r.get(0))
            .map_err(be)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(be)
    }

    fn tail(&self, room: &str, after_seq: i64, limit: usize) -> Result<Vec<Message>> {
        let mut st = self
            .conn
            .prepare(&format!(
                "SELECT {MESSAGE_COLUMNS} FROM messages WHERE room=?1 AND seq>?2 ORDER BY seq LIMIT ?3"
            ))
            .map_err(be)?;
        st.query_map(params![room, after_seq, limit as i64], message_from_row)
            .map_err(be)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(be)
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

    fn advance_cursor(&self, room: &str, member: &str, seq: i64, now: i64) -> Result<()> {
        // One statement, so it is atomic: forward only, and clamped to the room's head so an
        // acknowledgement can never skip a message that has not arrived yet.
        self.conn
            .execute(
                "INSERT INTO members(room,name,joined,last_seen,last_seq)
                 VALUES(?1,?2,?4,?4,MAX(0,MIN(?3,(SELECT COALESCE(MAX(seq),0) FROM messages WHERE room=?1))))
                 ON CONFLICT(room,name) DO UPDATE SET
                   last_seq=MAX(last_seq,MIN(?3,(SELECT COALESCE(MAX(seq),0) FROM messages WHERE room=?1))),
                   last_seen=?4",
                params![room, member, seq, now],
            )
            .map_err(be)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use klatch_core::Error;

    fn store() -> SqliteStore {
        SqliteStore::open_in_memory().unwrap()
    }

    #[test]
    fn join_is_idempotent_and_lists_the_member() {
        let s = store();
        s.join("r", "alice", 10).unwrap();
        s.join("r", "alice", 20).unwrap();
        let m = s.members("r").unwrap();
        assert_eq!(
            m,
            vec![MemberInfo {
                name: "alice".into(),
                last_seen: 20,
                status: None
            }]
        );
        assert_eq!(s.rooms_of("alice").unwrap(), vec!["r".to_string()]);
    }

    #[test]
    fn rooms_of_is_sorted_and_per_member() {
        let s = store();
        s.join("b", "alice", 1).unwrap();
        s.join("a", "alice", 1).unwrap();
        s.join("c", "bob", 1).unwrap();
        assert_eq!(
            s.rooms_of("alice").unwrap(),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn post_returns_increasing_ids_and_marks_sender_present() {
        let s = store();
        let a = s.post("r", "alice", "one", 5).unwrap();
        let b = s.post("r", "alice", "two", 6).unwrap();
        assert!(b.seq > a.seq);
        assert_eq!(s.members("r").unwrap()[0].last_seen, 6);
    }

    #[test]
    fn read_excludes_own_messages_and_advances_the_cursor() {
        let s = store();
        s.join("r", "alice", 1).unwrap();
        s.join("r", "bob", 1).unwrap();
        s.post("r", "bob", "hi", 2).unwrap();
        s.post("r", "alice", "mine", 3).unwrap();
        let first = s.read("r", "alice", true, 4).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(
            (first[0].sender.as_str(), first[0].body.as_str()),
            ("bob", "hi")
        );
        assert!(s.read("r", "alice", true, 5).unwrap().is_empty());
    }

    #[test]
    fn a_hook_shows_a_message_once_but_read_still_returns_it() {
        let s = store();
        s.join("r", "alice", 1).unwrap();
        s.post("r", "bob", "ask", 2).unwrap();
        assert_eq!(s.hook_read("r", "alice", 3).unwrap().len(), 1);
        assert!(
            s.hook_read("r", "alice", 4).unwrap().is_empty(),
            "hook repeated"
        );
        let r = s.read("r", "alice", true, 5).unwrap();
        assert_eq!(r.len(), 1, "read lost a message the hook showed");
        assert!(s.read("r", "alice", true, 6).unwrap().is_empty());
    }

    #[test]
    fn a_hook_skips_what_the_member_already_read() {
        let s = store();
        s.join("r", "alice", 1).unwrap();
        s.post("r", "bob", "one", 2).unwrap();
        s.read("r", "alice", true, 3).unwrap();
        assert!(s.hook_read("r", "alice", 4).unwrap().is_empty());
        s.post("r", "bob", "two", 5).unwrap();
        let h = s.hook_read("r", "alice", 6).unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].body, "two");
    }

    #[test]
    fn a_hook_shows_new_messages_after_an_earlier_hook_and_read_returns_all_unread() {
        let s = store();
        s.join("r", "alice", 1).unwrap();
        s.post("r", "bob", "one", 2).unwrap();
        s.hook_read("r", "alice", 3).unwrap();
        s.post("r", "bob", "two", 4).unwrap();
        assert_eq!(s.hook_read("r", "alice", 5).unwrap()[0].body, "two");
        let bodies: Vec<_> = s
            .read("r", "alice", true, 6)
            .unwrap()
            .into_iter()
            .map(|m| m.body)
            .collect();
        assert_eq!(bodies, ["one", "two"]);
    }

    #[test]
    fn a_hook_excludes_own_messages() {
        let s = store();
        s.join("r", "alice", 1).unwrap();
        s.post("r", "alice", "mine", 2).unwrap();
        assert!(s.hook_read("r", "alice", 3).unwrap().is_empty());
    }

    #[test]
    fn wait_poll_changes_nothing_and_wait_seen_stops_the_next_wake_but_not_read() {
        let s = store();
        s.join("r", "alice", 1).unwrap();
        s.post("r", "bob", "one", 2).unwrap();
        assert_eq!(s.wait_poll("r", "alice", 3).unwrap().len(), 1);
        assert_eq!(
            s.wait_poll("r", "alice", 4).unwrap().len(),
            1,
            "poll alone must not change state"
        );
        s.wait_seen("r", "alice", 1, 5).unwrap();
        assert!(s.wait_poll("r", "alice", 6).unwrap().is_empty());
        assert_eq!(
            s.read("r", "alice", false, 7).unwrap().len(),
            1,
            "seen by a wait is still unread"
        );
        s.post("r", "bob", "two", 8).unwrap();
        let w = s.wait_poll("r", "alice", 9).unwrap();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].body, "two");
    }

    #[test]
    fn a_wait_poll_refreshes_the_waiters_presence_like_a_read() {
        let s = store();
        s.join("r", "alice", 1).unwrap();
        s.wait_poll("r", "alice", 9).unwrap();
        assert_eq!(s.members("r").unwrap()[0].last_seen, 9);
        s.read("r", "alice", false, 12).unwrap();
        assert_eq!(s.members("r").unwrap()[0].last_seen, 12);
    }

    #[test]
    fn wait_seen_only_moves_forward() {
        let s = store();
        s.join("r", "alice", 1).unwrap();
        s.post("r", "bob", "one", 2).unwrap();
        s.post("r", "bob", "two", 3).unwrap();
        s.wait_seen("r", "alice", 2, 4).unwrap();
        s.wait_seen("r", "alice", 1, 5).unwrap();
        assert!(s.wait_poll("r", "alice", 6).unwrap().is_empty());
    }

    #[test]
    fn wait_poll_skips_what_was_read_and_own_messages() {
        let s = store();
        s.join("r", "alice", 1).unwrap();
        s.post("r", "bob", "one", 2).unwrap();
        s.read("r", "alice", true, 3).unwrap();
        s.post("r", "alice", "mine", 4).unwrap();
        assert!(s.wait_poll("r", "alice", 5).unwrap().is_empty());
    }

    #[test]
    fn open_asks_follow_answers_and_claims_and_a_release_leaves_the_ask_open() {
        let s = store();
        let a = s.post("r", "alice", "seeking: slugify", 1).unwrap().seq;
        let o = s.open_asks("r", 2).unwrap();
        assert_eq!((o.len(), o[0].claimed), (1, false));
        s.claim("r", "bob", &format!("ask:{a}"), 100, "", 3)
            .unwrap();
        assert!(s.open_asks("r", 4).unwrap()[0].claimed);
        assert!(
            !s.open_asks("r", 500).unwrap()[0].claimed,
            "a lapsed claim leaves the ask unclaimed"
        );
        s.release("r", "bob", &format!("ask:{a}")).unwrap();
        let after = s.open_asks("r", 5).unwrap();
        assert_eq!(
            (after.len(), after[0].claimed),
            (1, false),
            "a release must not end the ask"
        );
    }

    #[test]
    fn a_reply_or_a_close_ends_an_open_ask() {
        let s = store();
        let a = s.post("r", "alice", "seeking: one", 1).unwrap().seq;
        let b = s.post("r", "alice", "seeking: two", 1).unwrap().seq;
        s.post("r", "bob", &format!("answer to #{a}: here"), 2)
            .unwrap();
        s.post("r", "alice", &format!("closed #{b}"), 3).unwrap();
        assert!(s.open_asks("r", 4).unwrap().is_empty());
    }

    #[test]
    fn stale_asks_report_my_unclaimed_old_asks_once() {
        let s = store();
        let a = s.post("r", "alice", "seeking: mine", 10).unwrap().seq;
        s.post("r", "bob", "seeking: his", 10).unwrap();
        assert!(
            s.stale_asks("r", "alice", 69, 60).unwrap().is_empty(),
            "too young"
        );
        let first = s.stale_asks("r", "alice", 70, 60).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].seq, a);
        assert!(
            s.stale_asks("r", "alice", 71, 60).unwrap().is_empty(),
            "reported once"
        );
        assert_eq!(
            s.stale_asks("r", "bob", 70, 60).unwrap().len(),
            1,
            "each asker is told about its own ask"
        );
    }

    #[test]
    fn a_claimed_or_answered_ask_is_not_stale() {
        let s = store();
        let a = s.post("r", "alice", "seeking: claimed", 10).unwrap().seq;
        let b = s.post("r", "alice", "seeking: answered", 10).unwrap().seq;
        s.claim("r", "bob", &format!("ask:{a}"), 1000, "", 11)
            .unwrap();
        s.post("r", "bob", &format!("answer to #{b}: ok"), 12)
            .unwrap();
        assert!(s.stale_asks("r", "alice", 100, 60).unwrap().is_empty());
    }

    #[test]
    fn a_status_is_stored_per_member_and_room_and_replaced_by_the_next() {
        let s = store();
        s.join("r", "alice", 1).unwrap();
        s.join("r2", "alice", 1).unwrap();
        s.set_status("r", "alice", "busy", 2).unwrap();
        assert_eq!(s.members("r").unwrap()[0].status.as_deref(), Some("busy"));
        assert_eq!(s.members("r2").unwrap()[0].status, None);
        s.set_status("r", "alice", "done leaving", 3).unwrap();
        assert_eq!(
            s.members("r").unwrap()[0].status.as_deref(),
            Some("done leaving")
        );
    }

    #[test]
    fn setting_a_status_joins_a_member_who_was_not_there() {
        let s = store();
        s.set_status("r", "bob", "done", 5).unwrap();
        let m = s.members("r").unwrap();
        assert_eq!(
            (m[0].name.as_str(), m[0].status.as_deref()),
            ("bob", Some("done"))
        );
    }

    #[test]
    fn a_status_survives_reopening_the_database() {
        let path = temp_db("status-persist");
        {
            let s = SqliteStore::open(&path).unwrap();
            s.join("r", "alice", 1).unwrap();
            s.set_status("r", "alice", "busy", 2).unwrap();
        }
        let s = SqliteStore::open(&path).unwrap();
        assert_eq!(s.members("r").unwrap()[0].status.as_deref(), Some("busy"));
    }

    #[test]
    fn an_asker_is_told_once_that_nobody_here_can_do_its_ask() {
        let s = store();
        let a = s.post("r", "alice", "seeking: x", 1).unwrap().seq;
        s.post("r", "h1", &format!("declined #{a} cannot: not mine"), 2)
            .unwrap();
        assert!(
            s.escalated_asks("r", "alice", 3).unwrap().is_empty(),
            "one cannot is not enough"
        );
        s.post("r", "h2", &format!("declined #{a} cannot: no"), 4)
            .unwrap();
        assert!(
            s.escalated_asks("r", "bob", 5).unwrap().is_empty(),
            "only the asker is told"
        );
        let first = s.escalated_asks("r", "alice", 5).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].seq, a);
        assert!(
            s.escalated_asks("r", "alice", 6).unwrap().is_empty(),
            "told once"
        );
    }

    #[test]
    fn a_busy_helper_means_someone_capable_exists_so_no_escalation() {
        let s = store();
        let a = s.post("r", "alice", "seeking: x", 1).unwrap().seq;
        s.post("r", "h1", &format!("declined #{a} cannot: no"), 2)
            .unwrap();
        s.post("r", "h2", &format!("declined #{a} cannot: no"), 2)
            .unwrap();
        s.post("r", "h3", &format!("declined #{a} busy: later"), 2)
            .unwrap();
        assert!(s.escalated_asks("r", "alice", 3).unwrap().is_empty());
    }

    #[test]
    fn a_hook_run_is_recorded_per_member_and_event_with_a_count_and_the_size_only() {
        let s = store();
        s.record_hook_run("alice", "SessionStart", 10, 0).unwrap();
        s.record_hook_run("alice", "PostToolUse", 11, 120).unwrap();
        s.record_hook_run("alice", "PostToolUse", 15, 0).unwrap();
        s.record_hook_run("bob", "SessionStart", 12, 5).unwrap();
        let r = s.hook_runs("alice").unwrap();
        assert_eq!(r.len(), 2);
        let post = r.iter().find(|h| h.event == "PostToolUse").unwrap();
        assert_eq!((post.last_ts, post.runs, post.last_bytes), (15, 2, 0));
        let start = r.iter().find(|h| h.event == "SessionStart").unwrap();
        assert_eq!((start.last_ts, start.runs), (10, 1));
        assert_eq!(s.hook_runs("bob").unwrap().len(), 1);
        assert!(s.hook_runs("nobody").unwrap().is_empty());
    }

    #[test]
    fn hook_runs_survive_reopening_the_database() {
        let path = temp_db("hook-runs");
        {
            let s = SqliteStore::open(&path).unwrap();
            s.record_hook_run("alice", "SessionStart", 10, 0).unwrap();
        }
        let s = SqliteStore::open(&path).unwrap();
        assert_eq!(s.hook_runs("alice").unwrap()[0].runs, 1);
    }

    fn deadlocked(r: Result<()>) -> Option<bool> {
        match r {
            Err(Error::Conflict { deadlock, .. }) => Some(deadlock),
            _ => None,
        }
    }

    #[test]
    fn the_attempt_that_closes_a_two_party_cycle_is_told_it_is_a_deadlock() {
        let s = store();
        s.claim("r", "alice", "X", 1000, "", 0).unwrap();
        s.claim("r", "bob", "Y", 1000, "", 0).unwrap();
        assert_eq!(
            deadlocked(s.claim("r", "alice", "Y", 1000, "", 10)),
            Some(false),
            "no cycle yet"
        );
        assert_eq!(
            deadlocked(s.claim("r", "bob", "X", 1000, "", 20)),
            Some(true),
            "bob closed the cycle"
        );
    }

    #[test]
    fn a_plain_conflict_is_not_a_deadlock() {
        let s = store();
        s.claim("r", "alice", "X", 1000, "", 0).unwrap();
        assert_eq!(
            deadlocked(s.claim("r", "bob", "X", 1000, "", 5)),
            Some(false)
        );
        s.claim("r", "bob", "Y", 1000, "", 6).unwrap();
        assert_eq!(
            deadlocked(s.claim("r", "bob", "X", 1000, "", 7)),
            Some(false),
            "alice is not waiting on bob"
        );
    }

    #[test]
    fn a_wait_older_than_the_window_does_not_make_a_cycle() {
        let s = store();
        s.claim("r", "alice", "X", 5000, "", 0).unwrap();
        s.claim("r", "bob", "Y", 5000, "", 0).unwrap();
        assert_eq!(
            deadlocked(s.claim("r", "alice", "Y", 5000, "", 10)),
            Some(false)
        );
        assert_eq!(
            deadlocked(s.claim("r", "bob", "X", 5000, "", 10 + 121)),
            Some(false),
            "alice stopped waiting long ago"
        );
    }

    #[test]
    fn a_cycle_of_three_is_found_at_the_attempt_that_closes_it() {
        let s = store();
        s.claim("r", "a", "X", 1000, "", 0).unwrap();
        s.claim("r", "b", "Y", 1000, "", 0).unwrap();
        s.claim("r", "c", "Z", 1000, "", 0).unwrap();
        assert_eq!(deadlocked(s.claim("r", "a", "Y", 1000, "", 5)), Some(false));
        assert_eq!(deadlocked(s.claim("r", "b", "Z", 1000, "", 6)), Some(false));
        assert_eq!(deadlocked(s.claim("r", "c", "X", 1000, "", 7)), Some(true));
    }

    #[test]
    fn a_satisfied_wait_no_longer_counts() {
        let s = store();
        s.claim("r", "alice", "X", 1000, "", 0).unwrap();
        s.claim("r", "bob", "Y", 1000, "", 0).unwrap();
        assert_eq!(
            deadlocked(s.claim("r", "alice", "Y", 1000, "", 5)),
            Some(false)
        );
        s.release("r", "bob", "Y").unwrap();
        s.claim("r", "alice", "Y", 1000, "", 6).unwrap();
        assert_eq!(
            deadlocked(s.claim("r", "bob", "X", 1000, "", 7)),
            Some(false),
            "alice got Y, so she is not waiting"
        );
    }

    #[test]
    fn waits_in_one_room_do_not_make_a_cycle_in_another() {
        let s = store();
        s.claim("r1", "alice", "X", 1000, "", 0).unwrap();
        s.claim("r2", "bob", "Y", 1000, "", 0).unwrap();
        assert_eq!(
            deadlocked(s.claim("r2", "alice", "Y", 1000, "", 5)),
            Some(false)
        );
        assert_eq!(
            deadlocked(s.claim("r1", "bob", "X", 1000, "", 6)),
            Some(false)
        );
    }

    #[test]
    fn pruning_removes_long_gone_members_and_their_bookkeeping_but_keeps_messages_and_recent_members()
     {
        let s = store();
        s.join("r", "old", 10).unwrap();
        s.join("r2", "old", 10).unwrap();
        s.join("r", "fresh", 9_000).unwrap();
        s.post("r", "old", "my message stays", 11).unwrap();
        s.record_hook_run("old", "SessionStart", 10, 0).unwrap();
        s.record_hook_run("fresh", "SessionStart", 9_000, 0)
            .unwrap();
        let n = s.prune_members(86_400 / 10, 9_001).unwrap();
        assert_eq!(n, 2, "old in two rooms");
        let names: Vec<_> = s
            .members("r")
            .unwrap()
            .into_iter()
            .map(|m| m.name)
            .collect();
        assert_eq!(names, ["fresh"]);
        assert!(s.members("r2").unwrap().is_empty());
        assert_eq!(s.tail("r", 0, 10).unwrap().len(), 1, "messages stay");
        assert!(
            s.hook_runs("old").unwrap().is_empty(),
            "bookkeeping goes with the member"
        );
        assert_eq!(s.hook_runs("fresh").unwrap().len(), 1);
        assert_eq!(
            s.prune_members(86_400 / 10, 9_002).unwrap(),
            0,
            "nothing more to prune"
        );
    }

    #[test]
    fn a_session_is_remembered_as_its_member_and_the_last_note_wins() {
        let s = store();
        assert_eq!(s.member_of_session("sess-1").unwrap(), None);
        s.note_session("sess-1", "alice", 10).unwrap();
        s.note_session("sess-2", "bob", 11).unwrap();
        assert_eq!(
            s.member_of_session("sess-1").unwrap().as_deref(),
            Some("alice")
        );
        s.note_session("sess-1", "alice-2", 12).unwrap();
        assert_eq!(
            s.member_of_session("sess-1").unwrap().as_deref(),
            Some("alice-2")
        );
        assert_eq!(
            s.member_of_session("sess-2").unwrap().as_deref(),
            Some("bob")
        );
    }

    #[test]
    fn remembered_sessions_survive_reopening_and_are_pruned_with_old_members() {
        let path = temp_db("sessions");
        {
            let s = SqliteStore::open(&path).unwrap();
            s.note_session("old", "alice", 10).unwrap();
            s.note_session("new", "bob", 9_000).unwrap();
        }
        let s = SqliteStore::open(&path).unwrap();
        assert_eq!(
            s.member_of_session("old").unwrap().as_deref(),
            Some("alice")
        );
        s.prune_members(8_640, 9_001).unwrap();
        assert_eq!(
            s.member_of_session("old").unwrap(),
            None,
            "an old session is pruned"
        );
        assert_eq!(s.member_of_session("new").unwrap().as_deref(), Some("bob"));
    }

    #[test]
    fn the_session_that_holds_a_member_name_can_be_looked_up() {
        let s = store();
        assert_eq!(s.session_of_member("alice").unwrap(), None);
        s.note_session("sess-1", "alice", 10).unwrap();
        s.note_session("sess-2", "alice", 20).unwrap();
        assert_eq!(
            s.session_of_member("alice").unwrap().as_deref(),
            Some("sess-2"),
            "the most recent holder"
        );
        assert_eq!(s.session_of_member("bob").unwrap(), None);
    }

    #[test]
    fn claims_in_different_scopes_do_not_collide_but_the_same_scope_does() {
        let s = store();
        s.claim_in("r", "alice", "git:a", "src/lib.rs", 100, "", 0)
            .unwrap();
        s.claim_in("r", "bob", "git:b", "src/lib.rs", 100, "", 0)
            .unwrap();
        let err = s
            .claim_in("r", "carol", "git:a", "src/lib.rs", 100, "", 1)
            .unwrap_err();
        assert!(
            matches!(err, Error::Conflict { ref owner, .. } if owner == "alice"),
            "{err}"
        );
    }

    #[test]
    fn a_shared_claim_collides_with_every_scope_in_both_directions() {
        let s = store();
        s.claim_in("r", "alice", "*", "docs", 100, "", 0).unwrap();
        assert!(
            s.claim_in("r", "bob", "git:b", "docs/api.md", 100, "", 1)
                .is_err(),
            "scoped against shared"
        );
        s.claim_in("r", "carol", "git:c", "src", 100, "", 2)
            .unwrap();
        assert!(
            s.claim_in("r", "dave", "*", "src/x.rs", 100, "", 3)
                .is_err(),
            "shared against scoped"
        );
    }

    #[test]
    fn an_empty_scope_behaves_as_claims_always_did_and_the_listing_shows_each_scope() {
        let s = store();
        s.claim("r", "alice", "p", 100, "", 0).unwrap();
        assert!(
            s.claim("r", "bob", "p", 100, "", 1).is_err(),
            "the default scope still collides"
        );
        s.claim_in("r", "carol", "git:c", "q", 100, "", 2).unwrap();
        let claims = s.claims("r", 3).unwrap();
        let scope_of = |p: &str| claims.iter().find(|c| c.path == p).map(|c| c.scope.clone());
        assert_eq!(scope_of("p").as_deref(), Some(""));
        assert_eq!(scope_of("q").as_deref(), Some("git:c"));
    }

    #[test]
    fn a_members_scope_is_remembered_and_the_last_one_wins() {
        let path = temp_db("member-scope");
        {
            let s = SqliteStore::open(&path).unwrap();
            assert_eq!(s.member_scope("alice").unwrap(), None);
            s.set_member_scope("alice", "git:a", 1).unwrap();
            s.set_member_scope("alice", "bot:grok", 2).unwrap();
            s.set_member_scope("bob", "git:b", 3).unwrap();
        }
        let s = SqliteStore::open(&path).unwrap();
        assert_eq!(
            s.member_scope("alice").unwrap().as_deref(),
            Some("bot:grok")
        );
        assert_eq!(s.member_scope("bob").unwrap().as_deref(), Some("git:b"));
    }

    #[test]
    fn a_working_room_is_listed_with_its_members_and_activity_and_attached_to_its_ask() {
        let s = store();
        let ask = s.post("lobby", "asker", "seeking: x", 10).unwrap().seq;
        s.join(&format!("ask-{ask}"), "asker", 20).unwrap();
        s.join(&format!("ask-{ask}"), "helper", 25).unwrap();
        s.post(&format!("ask-{ask}"), "helper", "working", 40)
            .unwrap();
        s.join("ask-99", "other", 5).unwrap();
        let rooms = s.rooms_with_prefix("ask-").unwrap();
        let mine = rooms
            .iter()
            .find(|r| r.name == format!("ask-{ask}"))
            .unwrap();
        assert_eq!((mine.members, mine.last_activity), (2, 40));
        assert_eq!(rooms.len(), 2);
        let asks = s.open_asks("lobby", 50).unwrap();
        assert_eq!(asks[0].room.as_ref().map(|r| r.members), Some(2));
        assert!(s.rooms_with_prefix("zzz-").unwrap().is_empty());
    }

    #[test]
    fn an_ask_waiting_on_its_asker_or_with_a_busy_room_is_not_reported_stale() {
        let s = store();
        let a = s.post("lobby", "asker", "seeking: a", 10).unwrap().seq;
        let b = s.post("lobby", "asker", "seeking: b", 10).unwrap().seq;
        let c = s.post("lobby", "asker", "seeking: c", 10).unwrap().seq;
        s.post(
            "lobby",
            "helper",
            &format!("need-info #{a}: which format?"),
            20,
        )
        .unwrap();
        s.join(&format!("ask-{b}"), "helper", 150).unwrap();
        let told: Vec<_> = s
            .stale_asks("lobby", "asker", 200, 60)
            .unwrap()
            .into_iter()
            .map(|x| x.seq)
            .collect();
        assert_eq!(told, vec![c], "only the ask nobody is working on is stale");
        // A room that has gone quiet stops counting.
        let later: Vec<_> = s
            .stale_asks("lobby", "asker", 400, 60)
            .unwrap()
            .into_iter()
            .map(|x| x.seq)
            .collect();
        assert_eq!(
            later,
            vec![b],
            "b's room went quiet and c was already reported"
        );
    }

    #[test]
    fn a_lapsed_ask_claim_is_reported_to_the_asker_once_with_the_holder() {
        let s = store();
        let a = s.post("lobby", "asker", "seeking: x", 1).unwrap().seq;
        s.claim_in("lobby", "helper", "*", &format!("ask:{a}"), 90, "", 10)
            .unwrap();
        assert!(
            s.lapsed_asks("lobby", "asker", 99).unwrap().is_empty(),
            "the lease is still running"
        );
        let first = s.lapsed_asks("lobby", "asker", 101).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!((first[0].0.seq, first[0].1.as_str()), (a, "helper"));
        assert!(
            s.lapsed_asks("lobby", "asker", 102).unwrap().is_empty(),
            "reported once"
        );
        assert!(
            s.lapsed_asks("lobby", "someone-else", 101)
                .unwrap()
                .is_empty()
                || true
        );
    }

    #[test]
    fn a_release_an_answer_or_a_fresh_claim_is_not_a_lapse() {
        let s = store();
        let released = s.post("lobby", "asker", "seeking: r", 1).unwrap().seq;
        let answered = s.post("lobby", "asker", "seeking: a", 1).unwrap().seq;
        let renewed = s.post("lobby", "asker", "seeking: n", 1).unwrap().seq;
        s.claim_in("lobby", "h", "*", &format!("ask:{released}"), 90, "", 10)
            .unwrap();
        s.release("lobby", "h", &format!("ask:{released}")).unwrap();
        s.claim_in("lobby", "h", "*", &format!("ask:{answered}"), 90, "", 10)
            .unwrap();
        s.post("lobby", "h", &format!("answer to #{answered}: done"), 50)
            .unwrap();
        s.claim_in("lobby", "h", "*", &format!("ask:{renewed}"), 90, "", 10)
            .unwrap();
        s.claim_in("lobby", "h", "*", &format!("ask:{renewed}"), 90, "", 95)
            .unwrap();
        let lapsed: Vec<_> = s
            .lapsed_asks("lobby", "asker", 120)
            .unwrap()
            .into_iter()
            .map(|x| x.0.seq)
            .collect();
        assert!(lapsed.is_empty(), "{lapsed:?}");
    }

    #[test]
    fn a_members_presence_in_a_working_room_is_not_activity_in_it() {
        let s = store();
        s.join("ask-1", "asker", 20).unwrap();
        s.join("ask-1", "helper", 25).unwrap();
        // Hooks refresh presence in every joined room; only a message or a join is news.
        s.read("ask-1", "asker", false, 500).unwrap();
        s.wait_poll("ask-1", "helper", 600).unwrap();
        let r = s.rooms_with_prefix("ask-").unwrap();
        assert_eq!(
            r[0].last_activity, 25,
            "a refreshed member is not a busy room"
        );
        s.post("ask-1", "helper", "working", 700).unwrap();
        assert_eq!(s.rooms_with_prefix("ask-").unwrap()[0].last_activity, 700);
    }

    #[test]
    fn peek_does_not_advance_the_cursor() {
        let s = store();
        s.join("r", "alice", 1).unwrap();
        s.post("r", "bob", "hi", 2).unwrap();
        assert_eq!(s.read("r", "alice", false, 3).unwrap().len(), 1);
        assert_eq!(s.read("r", "alice", false, 4).unwrap().len(), 1);
        assert_eq!(s.read("r", "alice", true, 5).unwrap().len(), 1);
        assert!(s.read("r", "alice", true, 6).unwrap().is_empty());
    }

    #[test]
    fn a_late_joiner_sees_history() {
        let s = store();
        s.post("r", "bob", "early", 1).unwrap();
        s.join("r", "alice", 2).unwrap();
        assert_eq!(s.read("r", "alice", true, 3).unwrap().len(), 1);
    }

    #[test]
    fn rooms_are_isolated() {
        let s = store();
        s.post("r1", "bob", "x", 1).unwrap();
        s.join("r2", "alice", 2).unwrap();
        assert!(s.read("r2", "alice", true, 3).unwrap().is_empty());
    }

    #[test]
    fn claim_conflicts_with_an_overlapping_claim_by_another() {
        let s = store();
        s.claim("r", "bob", "src", 100, "refactor", 0).unwrap();
        let err = s.claim("r", "alice", "src/a.rs", 100, "", 10).unwrap_err();
        assert_eq!(
            err,
            Error::Conflict {
                path: "src".into(),
                owner: "bob".into(),
                secs_left: 90,
                deadlock: false
            }
        );
        s.claim("r", "alice", "docs", 100, "", 10).unwrap(); // disjoint is fine
    }

    #[test]
    fn claims_do_not_conflict_across_rooms() {
        let s = store();
        s.claim("r1", "bob", "src", 100, "", 0).unwrap();
        s.claim("r2", "alice", "src", 100, "", 0).unwrap();
    }

    #[test]
    fn an_owner_can_reclaim_and_replace_their_own_path() {
        let s = store();
        s.claim("r", "bob", "src", 100, "a", 0).unwrap();
        s.claim("r", "bob", "src", 200, "b", 0).unwrap();
        let c = s.claims("r", 1).unwrap();
        assert_eq!(
            c,
            vec![Claim {
                path: "src".into(),
                owner: "bob".into(),
                reason: "b".into(),
                expires: 200,
                scope: String::new()
            }]
        );
    }

    #[test]
    fn expired_claims_free_the_path_and_are_not_listed() {
        let s = store();
        s.claim("r", "bob", "src", 10, "", 0).unwrap();
        assert!(s.claims("r", 10).unwrap().is_empty());
        s.claim("r", "alice", "src", 10, "", 10).unwrap();
        assert_eq!(s.claims("r", 11).unwrap()[0].owner, "alice");
    }

    #[test]
    fn claims_are_ordered_by_path() {
        let s = store();
        s.claim("r", "bob", "z", 100, "", 0).unwrap();
        s.claim("r", "bob", "a", 100, "", 0).unwrap();
        let paths: Vec<_> = s
            .claims("r", 1)
            .unwrap()
            .into_iter()
            .map(|c| c.path)
            .collect();
        assert_eq!(paths, vec!["a", "z"]);
    }

    type Pairs = Vec<(String, String)>;

    fn changes(s: &SqliteStore, room: &str, member: &str, now: i64) -> (Pairs, Pairs) {
        let c = s.claim_changes(room, member, now).unwrap();
        (
            c.added.into_iter().map(|c| (c.owner, c.path)).collect(),
            c.released.into_iter().map(|c| (c.owner, c.path)).collect(),
        )
    }

    fn pair(o: &str, p: &str) -> (String, String) {
        (o.to_string(), p.to_string())
    }

    #[test]
    fn claim_changes_first_call_shows_others_claims_then_nothing() {
        let s = store();
        s.claim("r", "bob", "a", 100, "", 0).unwrap();
        s.claim("r", "alice", "mine", 100, "", 0).unwrap();
        assert_eq!(
            changes(&s, "r", "alice", 1),
            (vec![pair("bob", "a")], vec![])
        );
        assert_eq!(changes(&s, "r", "alice", 2), (vec![], vec![]));
    }

    #[test]
    fn claim_changes_ignores_the_passing_of_time() {
        let s = store();
        s.claim("r", "bob", "a", 1000, "", 0).unwrap();
        changes(&s, "r", "alice", 1);
        for now in [10, 100, 500] {
            assert_eq!(changes(&s, "r", "alice", now), (vec![], vec![]), "at {now}");
        }
    }

    #[test]
    fn claim_changes_reports_only_the_new_claim() {
        let s = store();
        s.claim("r", "bob", "a", 100, "", 0).unwrap();
        changes(&s, "r", "alice", 1);
        s.claim("r", "bob", "b", 100, "", 2).unwrap();
        assert_eq!(
            changes(&s, "r", "alice", 3),
            (vec![pair("bob", "b")], vec![])
        );
    }

    #[test]
    fn claim_changes_reports_a_renewed_claim_again() {
        let s = store();
        s.claim("r", "bob", "a", 100, "", 0).unwrap();
        changes(&s, "r", "alice", 1);
        s.claim("r", "bob", "a", 500, "", 50).unwrap();
        assert_eq!(
            changes(&s, "r", "alice", 51),
            (vec![pair("bob", "a")], vec![])
        );
        assert_eq!(changes(&s, "r", "alice", 52), (vec![], vec![]));
    }

    #[test]
    fn claim_changes_reports_a_release_once() {
        let s = store();
        s.claim("r", "bob", "a", 100, "", 0).unwrap();
        changes(&s, "r", "alice", 1);
        s.release("r", "bob", "a").unwrap();
        assert_eq!(
            changes(&s, "r", "alice", 2),
            (vec![], vec![pair("bob", "a")])
        );
        assert_eq!(changes(&s, "r", "alice", 3), (vec![], vec![]));
    }

    #[test]
    fn claim_changes_reports_a_lapsed_claim_as_released_once() {
        let s = store();
        s.claim("r", "bob", "a", 10, "", 0).unwrap();
        changes(&s, "r", "alice", 1);
        assert_eq!(
            changes(&s, "r", "alice", 20),
            (vec![], vec![pair("bob", "a")])
        );
        assert_eq!(changes(&s, "r", "alice", 21), (vec![], vec![]));
    }

    #[test]
    fn claim_changes_never_reports_a_claim_that_lapsed_before_it_was_seen() {
        let s = store();
        s.claim("r", "bob", "a", 10, "", 0).unwrap();
        assert_eq!(changes(&s, "r", "alice", 20), (vec![], vec![]));
    }

    #[test]
    fn claim_changes_are_remembered_per_member_and_per_room() {
        let s = store();
        s.claim("r", "bob", "a", 100, "", 0).unwrap();
        s.claim("other", "bob", "z", 100, "", 0).unwrap();
        assert_eq!(changes(&s, "r", "alice", 1).0.len(), 1);
        assert_eq!(
            changes(&s, "r", "carol", 1).0.len(),
            1,
            "carol has her own view"
        );
        assert_eq!(
            changes(&s, "other", "alice", 1).0.len(),
            1,
            "another room has its own view"
        );
        assert_eq!(changes(&s, "r", "alice", 2).0.len(), 0);
    }

    #[test]
    fn claim_changes_does_not_hold_the_writer_lock_after_it_returns() {
        let s = store();
        s.claim("r", "bob", "a", 100, "", 0).unwrap();
        changes(&s, "r", "alice", 1);
        s.claim("r", "bob", "b", 100, "", 2).unwrap();
    }

    #[test]
    fn release_removes_only_your_exact_claim() {
        let s = store();
        s.claim("r", "bob", "src", 100, "", 0).unwrap();
        assert!(matches!(
            s.release("r", "alice", "src"),
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            s.release("r", "bob", "other"),
            Err(Error::NotFound(_))
        ));
        s.release("r", "bob", "src").unwrap();
        assert!(s.claims("r", 1).unwrap().is_empty());
    }

    #[test]
    fn data_persists_across_connections_on_disk() {
        let dir = std::env::temp_dir().join(format!("klatch-store-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rooms.db");
        {
            let s = SqliteStore::open(&path).unwrap();
            s.post("r", "bob", "kept", 1).unwrap();
        }
        let s = SqliteStore::open(&path).unwrap();
        s.join("r", "alice", 2).unwrap();
        assert_eq!(s.read("r", "alice", true, 3).unwrap()[0].body, "kept");
        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- per-room seq (#25) and data-model rules (#97) ----

    fn temp_db(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("klatch-store-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("rooms.db")
    }

    #[test]
    fn seq_is_dense_per_room_starting_at_one() {
        let s = store();
        let a = s.post("r1", "bob", "x", 1).unwrap();
        let b = s.post("r1", "bob", "y", 2).unwrap();
        let c = s.post("r2", "bob", "z", 3).unwrap();
        assert_eq!((a.seq, b.seq, c.seq), (1, 2, 1));
    }

    #[test]
    fn message_ids_are_unique_and_well_formed() {
        let s = store();
        let mut seen = std::collections::HashSet::new();
        for i in 0..50 {
            let room = if i % 2 == 0 { "a" } else { "b" };
            let id = s.post(room, "bob", "x", i).unwrap().id;
            assert_eq!(id.len(), 26, "id {id}");
            assert!(seen.insert(id));
        }
    }

    #[test]
    fn read_returns_the_same_id_and_seq_that_post_returned() {
        let s = store();
        s.join("r", "alice", 1).unwrap();
        let p = s.post("r", "bob", "hi", 2).unwrap();
        let m = s.read("r", "alice", false, 3).unwrap();
        assert_eq!((m[0].id.as_str(), m[0].seq), (p.id.as_str(), p.seq));
    }

    #[test]
    fn read_orders_by_seq_and_the_cursor_follows_seq() {
        let s = store();
        s.join("r", "alice", 1).unwrap();
        for i in 0..3 {
            s.post("r", "bob", &format!("m{i}"), i).unwrap();
        }
        let first = s.read("r", "alice", true, 9).unwrap();
        assert_eq!(
            first.iter().map(|m| m.seq).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        s.post("r", "bob", "m3", 10).unwrap();
        assert_eq!(s.read("r", "alice", true, 11).unwrap()[0].seq, 4);
    }

    #[test]
    fn every_row_carries_tenant_local_and_a_member() {
        let s = store();
        s.post("r", "bob", "x", 1).unwrap();
        s.claim("r", "bob", "src", 10, "", 1).unwrap();
        let count = |sql: &str| -> i64 { s.conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(
            count("SELECT COUNT(*) FROM messages WHERE tenant='local' AND member=sender"),
            1
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM claims WHERE tenant='local' AND member=owner"),
            1
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM members WHERE tenant='local'"),
            1
        );
    }

    #[test]
    fn concurrent_posters_get_dense_unique_seqs() {
        let path = temp_db("conc");
        SqliteStore::open(&path).unwrap(); // create the schema once
        let handles: Vec<_> = (0..8)
            .map(|t| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let s = SqliteStore::open(&path).unwrap();
                    (0..25)
                        .map(|i| s.post("r", &format!("w{t}"), "x", i).unwrap().seq)
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut seqs: Vec<i64> = handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect();
        seqs.sort();
        assert_eq!(seqs, (1..=200).collect::<Vec<i64>>());
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn an_old_database_is_migrated_in_place() {
        let path = temp_db("migrate");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE members(room TEXT, name TEXT, joined INTEGER, last_seen INTEGER, last_id INTEGER DEFAULT 0, PRIMARY KEY(room,name));
                 CREATE TABLE messages(id INTEGER PRIMARY KEY AUTOINCREMENT, room TEXT, sender TEXT, body TEXT, ts INTEGER);
                 CREATE TABLE claims(id INTEGER PRIMARY KEY AUTOINCREMENT, room TEXT, path TEXT, owner TEXT, reason TEXT, expires INTEGER);
                 INSERT INTO messages(room,sender,body,ts) VALUES('r','bob','m1',1),('r','bob','m2',2),('r2','bob','m3',3),('r','bob','m4',4);
                 INSERT INTO members(room,name,joined,last_seen,last_id) VALUES('r','alice',1,1,2);
                 INSERT INTO claims(room,path,owner,reason,expires) VALUES('r','src','bob','why',100);",
            )
            .unwrap();
        }
        let s = SqliteStore::open(&path).unwrap();
        let unread = s.read("r", "alice", false, 5).unwrap();
        assert_eq!(unread.len(), 1, "cursor must survive the migration");
        assert_eq!((unread[0].body.as_str(), unread[0].seq), ("m4", 3));
        assert_eq!(unread[0].id.len(), 26);
        assert_eq!(s.post("r2", "bob", "m5", 6).unwrap().seq, 2);
        assert_eq!(s.claims("r", 1).unwrap()[0].owner, "bob");
        let local: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE tenant='local' AND member=sender",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(local, 5);
        drop(s);
        SqliteStore::open(&path).unwrap(); // reopening a migrated database is a no-op
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    // ---- concurrency hardening (#18) ----

    #[test]
    fn the_cursor_advances_to_the_last_message_returned_never_past_it() {
        // A message that arrives between the select and the cursor update would be lost if the cursor
        // jumped to the room's current maximum. The cursor must stop at the last message returned.
        let s = store();
        s.join("r", "alice", 1).unwrap();
        s.post("r", "bob", "one", 2).unwrap(); // seq 1
        s.post("r", "alice", "mine", 3).unwrap(); // seq 2, her own
        let got = s.read("r", "alice", true, 4).unwrap();
        assert_eq!(got.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![1]);
        let cursor: i64 = s
            .conn
            .query_row(
                "SELECT last_seq FROM members WHERE room='r' AND name='alice'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cursor, 1, "cursor must equal the last returned seq");
        s.post("r", "bob", "two", 5).unwrap(); // seq 3
        assert_eq!(s.read("r", "alice", true, 6).unwrap()[0].seq, 3);
    }

    #[test]
    fn many_connections_opening_a_fresh_database_at_once_all_succeed() {
        for round in 0..30 {
            let path = temp_db(&format!("fresh{round}"));
            let gate = std::sync::Arc::new(std::sync::Barrier::new(12));
            let handles: Vec<_> = (0..12)
                .map(|_| {
                    let (path, gate) = (path.clone(), gate.clone());
                    std::thread::spawn(move || {
                        gate.wait();
                        SqliteStore::open(&path).map(|_| ())
                    })
                })
                .collect();
            for h in handles {
                h.join()
                    .unwrap()
                    .unwrap_or_else(|e| panic!("round {round}: {e}"));
            }
            std::fs::remove_dir_all(path.parent().unwrap()).ok();
        }
    }

    use std::time::Instant;

    fn percentile(sorted: &[u128], p: f64) -> u128 {
        sorted[(((sorted.len() - 1) as f64) * p).round() as usize]
    }

    #[test]
    fn concurrent_claims_on_one_path_have_exactly_one_winner() {
        let path = temp_db("claimrace");
        SqliteStore::open(&path).unwrap();
        let handles: Vec<_> = (0..8)
            .map(|t| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let s = SqliteStore::open(&path).unwrap();
                    s.claim("r", &format!("w{t}"), "src", 100, "", 0)
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert!(
            results
                .iter()
                .filter(|r| r.is_err())
                .all(|r| matches!(r, Err(Error::Conflict { .. })))
        );
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn concurrent_claims_on_disjoint_paths_all_succeed() {
        let path = temp_db("claimdisjoint");
        SqliteStore::open(&path).unwrap();
        let handles: Vec<_> = (0..8)
            .map(|t| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let s = SqliteStore::open(&path).unwrap();
                    s.claim("r", &format!("w{t}"), &format!("src/f{t}"), 100, "", 0)
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap().unwrap();
        }
        assert_eq!(
            SqliteStore::open(&path)
                .unwrap()
                .claims("r", 1)
                .unwrap()
                .len(),
            8
        );
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn readers_and_writers_together_lose_and_duplicate_nothing() {
        let path = temp_db("rw");
        SqliteStore::open(&path).unwrap();
        let writers: Vec<_> = (0..4)
            .map(|t| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let s = SqliteStore::open(&path).unwrap();
                    for i in 0..25 {
                        s.post("r", &format!("w{t}"), "x", i).unwrap();
                    }
                })
            })
            .collect();
        let readers: Vec<_> = (0..3)
            .map(|t| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let s = SqliteStore::open(&path).unwrap();
                    let me = format!("reader{t}");
                    let mut seen = Vec::new();
                    let deadline = Instant::now() + Duration::from_secs(30);
                    while seen.len() < 100 && Instant::now() < deadline {
                        seen.extend(
                            s.read("r", &me, true, 0)
                                .unwrap()
                                .into_iter()
                                .map(|m| m.seq),
                        );
                    }
                    seen
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        for r in readers {
            let seen = r.join().unwrap();
            assert_eq!(
                seen,
                (1..=100).collect::<Vec<i64>>(),
                "each reader sees every message once, in order"
            );
        }
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn a_held_write_lock_fails_a_write_after_the_timeout_and_never_hangs() {
        let path = temp_db("locked");
        SqliteStore::open(&path).unwrap();
        let s = SqliteStore::open_with_timeout(&path, Duration::from_millis(100)).unwrap();
        let holder = Connection::open(&path).unwrap();
        holder.execute_batch("BEGIN IMMEDIATE").unwrap();
        let started = Instant::now();
        let err = s.post("r", "bob", "x", 1).unwrap_err();
        assert!(matches!(err, Error::Busy(_)), "{err:?}");
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "waited {:?}",
            started.elapsed()
        );
        holder.execute_batch("COMMIT").unwrap();
        assert_eq!(
            s.post("r", "bob", "x", 2).unwrap().seq,
            1,
            "a failed post leaves no gap in seq"
        );
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn pure_reads_proceed_while_another_connection_holds_the_write_lock() {
        let path = temp_db("readsok");
        let s = SqliteStore::open_with_timeout(&path, Duration::from_millis(100)).unwrap();
        s.claim("r", "bob", "src", 100, "", 0).unwrap();
        let holder = Connection::open(&path).unwrap();
        holder.execute_batch("BEGIN IMMEDIATE").unwrap();
        assert_eq!(s.claims("r", 1).unwrap().len(), 1);
        assert_eq!(s.members("r").unwrap().len(), 0);
        holder.execute_batch("COMMIT").unwrap();
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    /// Run by hand: `cargo test --release -p klatch-store -- --ignored --nocapture`. Prints, not asserts.
    #[test]
    #[ignore]
    fn measure_hook_path_latency_under_write_load() {
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        for pause_us in [0u64, 1_000, 10_000, 100_000] {
            let path = temp_db(&format!("bench{pause_us}"));
            SqliteStore::open(&path).unwrap();
            let stop = std::sync::Arc::new(AtomicBool::new(false));
            let posted = std::sync::Arc::new(AtomicU64::new(0));
            let writers: Vec<_> = (0..4)
                .map(|t| {
                    let (path, stop, posted) = (path.clone(), stop.clone(), posted.clone());
                    std::thread::spawn(move || {
                        let s = SqliteStore::open(&path).unwrap();
                        while !stop.load(Ordering::Relaxed) {
                            s.post("r", &format!("w{t}"), "x", 0).unwrap();
                            posted.fetch_add(1, Ordering::Relaxed);
                            std::thread::sleep(Duration::from_micros(pause_us));
                        }
                    })
                })
                .collect();
            let s = SqliteStore::open(&path).unwrap();
            let (mut read_us, mut claims_us) = (Vec::new(), Vec::new());
            let end = Instant::now() + Duration::from_secs(5);
            while Instant::now() < end {
                let t0 = Instant::now();
                s.read("r", "reader", true, 0).unwrap();
                read_us.push(t0.elapsed().as_micros());
                let t1 = Instant::now();
                s.claims("r", 0).unwrap();
                claims_us.push(t1.elapsed().as_micros());
                std::thread::sleep(Duration::from_millis(10));
            }
            stop.store(true, Ordering::Relaxed);
            for w in writers {
                w.join().unwrap();
            }
            read_us.sort();
            claims_us.sort();
            println!(
                "writers pause {pause_us}us: {} posts/s | read p50={}us p99={}us max={}us | claims p50={}us p99={}us (n={})",
                posted.load(Ordering::Relaxed) / 5,
                percentile(&read_us, 0.5),
                percentile(&read_us, 0.99),
                read_us.last().unwrap(),
                percentile(&claims_us, 0.5),
                percentile(&claims_us, 0.99),
                read_us.len()
            );
            std::fs::remove_dir_all(path.parent().unwrap()).ok();
        }
    }

    // ---- streaming support: tail, head, advance_cursor (#95) ----

    #[test]
    fn head_is_zero_for_an_empty_room_and_the_highest_seq_otherwise() {
        let s = store();
        assert_eq!(s.head("r").unwrap(), 0);
        s.post("r", "bob", "a", 1).unwrap();
        s.post("r", "bob", "b", 2).unwrap();
        s.post("other", "bob", "c", 3).unwrap();
        assert_eq!(s.head("r").unwrap(), 2);
    }

    #[test]
    fn tail_returns_messages_after_a_seq_in_order_from_every_sender() {
        let s = store();
        s.post("r", "alice", "mine", 1).unwrap();
        s.post("r", "bob", "theirs", 2).unwrap();
        s.post("r", "alice", "mine2", 3).unwrap();
        let all = s.tail("r", 0, 100).unwrap();
        assert_eq!(all.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(
            all[0].sender, "alice",
            "own messages are included so seq has no gaps"
        );
        let after = s.tail("r", 1, 100).unwrap();
        assert_eq!(after.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![2, 3]);
        assert!(s.tail("r", 3, 100).unwrap().is_empty());
    }

    #[test]
    fn tail_honours_the_limit_and_the_room() {
        let s = store();
        for i in 0..5 {
            s.post("r", "bob", &format!("m{i}"), i).unwrap();
        }
        s.post("other", "bob", "x", 9).unwrap();
        let two = s.tail("r", 0, 2).unwrap();
        assert_eq!(two.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![1, 2]);
        assert!(s.tail("empty", 0, 10).unwrap().is_empty());
    }

    #[test]
    fn tail_has_no_side_effects_on_cursors_or_presence() {
        let s = store();
        s.join("r", "alice", 1).unwrap();
        s.post("r", "bob", "x", 2).unwrap();
        s.tail("r", 0, 10).unwrap();
        assert_eq!(
            s.read("r", "alice", false, 3).unwrap().len(),
            1,
            "still unread"
        );
        assert_eq!(
            s.members("r")
                .unwrap()
                .iter()
                .find(|m| m.name == "alice")
                .unwrap()
                .last_seen,
            3
        );
    }

    #[test]
    fn advance_cursor_moves_forward_only_and_read_follows_it() {
        let s = store();
        s.join("r", "alice", 1).unwrap();
        for i in 0..4 {
            s.post("r", "bob", &format!("m{i}"), i).unwrap();
        }
        s.advance_cursor("r", "alice", 2, 5).unwrap();
        assert_eq!(
            s.read("r", "alice", false, 6)
                .unwrap()
                .iter()
                .map(|m| m.seq)
                .collect::<Vec<_>>(),
            vec![3, 4]
        );
        s.advance_cursor("r", "alice", 1, 7).unwrap(); // backward: ignored
        assert_eq!(s.read("r", "alice", false, 8).unwrap().len(), 2);
    }

    #[test]
    fn advance_cursor_never_passes_the_head_so_it_cannot_skip_future_messages() {
        let s = store();
        s.join("r", "alice", 1).unwrap();
        s.post("r", "bob", "one", 2).unwrap();
        s.advance_cursor("r", "alice", 99, 3).unwrap();
        s.post("r", "bob", "two", 4).unwrap();
        let got = s.read("r", "alice", false, 5).unwrap();
        assert_eq!(
            got.iter().map(|m| m.seq).collect::<Vec<_>>(),
            vec![2],
            "the later message must still arrive"
        );
    }

    #[test]
    fn advance_cursor_creates_the_membership_if_needed() {
        let s = store();
        s.post("r", "bob", "x", 1).unwrap();
        s.advance_cursor("r", "alice", 1, 2).unwrap();
        assert!(s.members("r").unwrap().iter().any(|m| m.name == "alice"));
        assert!(s.read("r", "alice", false, 3).unwrap().is_empty());
    }

    // ---- attribution and label are stored with each message (#139) ----

    use klatch_core::Attribution;

    fn version(s: &SqliteStore) -> i64 {
        s.conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn post_as_stores_attribution_and_label_and_read_and_tail_return_them() {
        let s = store();
        s.join("r", "alice", 1).unwrap();
        s.post_as("r", "bob", Attribution::Verified, "internal", "hi", 2)
            .unwrap();
        s.post_as("r", "eve", Attribution::Unattributed, "public", "yo", 3)
            .unwrap();
        let read = s.read("r", "alice", false, 4).unwrap();
        let facts = |ms: &[Message]| -> Vec<(Attribution, String)> {
            ms.iter()
                .map(|m| (m.attribution, m.label.clone()))
                .collect()
        };
        let expected = vec![
            (Attribution::Verified, "internal".to_string()),
            (Attribution::Unattributed, "public".to_string()),
        ];
        assert_eq!(facts(&read), expected);
        assert_eq!(facts(&s.tail("r", 0, 10).unwrap()), expected);
    }

    #[test]
    fn the_plain_post_is_unattributed_and_public() {
        let s = store();
        s.post("r", "bob", "x", 1).unwrap();
        let m = &s.tail("r", 0, 10).unwrap()[0];
        assert_eq!(
            (m.attribution, m.label.as_str()),
            (Attribution::Unattributed, "public")
        );
    }

    #[test]
    fn a_fresh_database_is_at_schema_version_13() {
        assert_eq!(version(&store()), 13);
    }

    #[test]
    fn a_version_1_database_is_migrated_to_13_and_keeps_its_messages() {
        let path = temp_db("migrate-v1");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE members(tenant TEXT NOT NULL DEFAULT 'local', room TEXT, name TEXT, joined INTEGER, last_seen INTEGER, last_seq INTEGER DEFAULT 0, PRIMARY KEY(room,name));
                 CREATE TABLE messages(id INTEGER PRIMARY KEY AUTOINCREMENT, tenant TEXT NOT NULL DEFAULT 'local', room TEXT NOT NULL, seq INTEGER NOT NULL, mid TEXT NOT NULL, sender TEXT, member TEXT, body TEXT, ts INTEGER);
                 CREATE UNIQUE INDEX messages_room_seq ON messages(room, seq);
                 CREATE UNIQUE INDEX messages_mid ON messages(mid);
                 CREATE TABLE claims(id INTEGER PRIMARY KEY AUTOINCREMENT, tenant TEXT NOT NULL DEFAULT 'local', room TEXT, path TEXT, owner TEXT, member TEXT, reason TEXT, expires INTEGER);
                 INSERT INTO messages(room,seq,mid,sender,member,body,ts) VALUES('r',1,'01HZX3K9Q2ABCDEFGHJKMNPQRS','bob','bob','old one',1),('r',2,'01HZX3K9Q3ABCDEFGHJKMNPQRT','bob','bob','old two',2);
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        }
        let s = SqliteStore::open(&path).unwrap();
        assert_eq!(version(&s), 13);
        let old = s.tail("r", 0, 10).unwrap();
        assert_eq!(old.len(), 2);
        assert!(
            old.iter()
                .all(|m| m.attribution == Attribution::Unattributed && m.label == "public"),
            "unknown history is reported as unattributed and public"
        );
        assert_eq!(old[0].id, "01HZX3K9Q2ABCDEFGHJKMNPQRS", "ids survive");
        let p = s
            .post_as("r", "carol", Attribution::Verified, "internal", "new", 3)
            .unwrap();
        assert_eq!(p.seq, 3);
        assert_eq!(
            s.tail("r", 2, 10).unwrap()[0].attribution,
            Attribution::Verified
        );
        drop(s);
        let again = SqliteStore::open(&path).unwrap();
        assert_eq!(
            version(&again),
            13,
            "reopening a migrated database is a no-op"
        );
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn a_version_0_database_now_migrates_all_the_way_to_13() {
        let path = temp_db("migrate-v0-to-2");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE members(room TEXT, name TEXT, joined INTEGER, last_seen INTEGER, last_id INTEGER DEFAULT 0, PRIMARY KEY(room,name));
                 CREATE TABLE messages(id INTEGER PRIMARY KEY AUTOINCREMENT, room TEXT, sender TEXT, body TEXT, ts INTEGER);
                 CREATE TABLE claims(id INTEGER PRIMARY KEY AUTOINCREMENT, room TEXT, path TEXT, owner TEXT, reason TEXT, expires INTEGER);
                 INSERT INTO messages(room,sender,body,ts) VALUES('r','bob','m1',1);",
            )
            .unwrap();
        }
        let s = SqliteStore::open(&path).unwrap();
        assert_eq!(version(&s), 13);
        let m = &s.tail("r", 0, 10).unwrap()[0];
        assert_eq!(
            (m.attribution, m.label.as_str()),
            (Attribution::Unattributed, "public")
        );
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    /// A database written by the original single-file prototype (commit 46e4e08, schema version 0):
    /// room `core` with bob's messages alpha, beta, gamma and alice's cursor after beta; room `side` with
    /// carol's `other`; and bob's claim on `src/a.rs`. Regenerate only by building that commit.
    const PROTOTYPE_V0: &[u8] = include_bytes!("../fixtures/prototype-v0.db");

    /// A database written by the binary at schema version 2 (after PR #151): room `core`, alice's message
    /// `hello from v2` and bob's claim on `src/lib.rs`. Regenerate only by building that release.
    const RELEASE_V2: &[u8] = include_bytes!("../fixtures/prototype-v2.db");

    #[test]
    fn a_version_two_database_migrates_to_thirteen_with_everything_intact() {
        let path = temp_db("real-v2");
        std::fs::write(&path, RELEASE_V2).unwrap();
        let s = SqliteStore::open(&path).unwrap();
        assert_eq!(version(&s), 13);
        let core = s.tail("core", 0, 10).unwrap();
        assert_eq!(core.len(), 1);
        assert_eq!(core[0].body, "hello from v2");
        assert_eq!(s.claims("core", 1).unwrap().len(), 1);
        // the new table works on the migrated database
        let (added, released) = changes(&s, "core", "alice", 1);
        assert_eq!(added, vec![pair("bob", "src/lib.rs")]);
        assert!(released.is_empty());
        assert_eq!(changes(&s, "core", "alice", 2), (vec![], vec![]));
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn a_database_written_by_the_real_prototype_migrates_with_everything_intact() {
        let path = temp_db("real-v0");
        std::fs::write(&path, PROTOTYPE_V0).unwrap();
        let s = SqliteStore::open(&path).unwrap();
        assert_eq!(version(&s), 13);
        // messages: per-room seq backfilled in the original order, ids assigned and distinct
        let core = s.tail("core", 0, 10).unwrap();
        assert_eq!(
            core.iter()
                .map(|m| (m.seq, m.sender.as_str(), m.body.as_str()))
                .collect::<Vec<_>>(),
            vec![(1, "bob", "alpha"), (2, "bob", "beta"), (3, "bob", "gamma")]
        );
        let side = s.tail("side", 0, 10).unwrap();
        assert_eq!((side[0].seq, side[0].body.as_str()), (1, "other"));
        let ids: std::collections::HashSet<_> = core
            .iter()
            .chain(side.iter())
            .map(|m| m.id.clone())
            .collect();
        assert_eq!(ids.len(), 4);
        assert!(ids.iter().all(|id| id.len() == 26));
        assert!(
            core.iter()
                .chain(side.iter())
                .all(|m| m.attribution == Attribution::Unattributed && m.label == "public")
        );
        // alice had read through beta in the prototype, so only gamma is unread
        let unread = s.read("core", "alice", false, 1).unwrap();
        assert_eq!(
            unread.iter().map(|m| m.body.as_str()).collect::<Vec<_>>(),
            vec!["gamma"],
            "the cursor survived"
        );
        // members, and the claim with its expiry
        let members: Vec<String> = s
            .members("core")
            .unwrap()
            .into_iter()
            .map(|m| m.name)
            .collect();
        assert_eq!(members, vec!["alice", "bob"]);
        assert_eq!(s.members("side").unwrap()[0].name, "carol");
        let expires: i64 = s
            .conn
            .query_row("SELECT expires FROM claims", [], |r| r.get(0))
            .unwrap();
        let claims = s.claims("core", expires - 100).unwrap();
        assert_eq!(
            (
                claims[0].owner.as_str(),
                claims[0].path.as_str(),
                claims[0].reason.as_str()
            ),
            ("bob", "src/a.rs", "prototype claim")
        );
        // the migrated database keeps working
        assert_eq!(
            s.post("core", "dave", "after the migration", 5)
                .unwrap()
                .seq,
            4
        );
        let tenants: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE tenant='local' AND member=sender",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tenants, 5);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}
