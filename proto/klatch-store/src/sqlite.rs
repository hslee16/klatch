use crate::Store;
use crate::ids::new_id;
use klatch_core::{Claim, Error, MemberInfo, Message, Posted, Result, overlaps};
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

const SCHEMA_VERSION: i64 = 2;

const SCHEMA: &str = "CREATE TABLE members(tenant TEXT NOT NULL DEFAULT 'local', room TEXT, name TEXT, joined INTEGER, last_seen INTEGER, last_seq INTEGER DEFAULT 0, PRIMARY KEY(room,name));
 CREATE TABLE messages(id INTEGER PRIMARY KEY AUTOINCREMENT, tenant TEXT NOT NULL DEFAULT 'local', room TEXT NOT NULL, seq INTEGER NOT NULL, mid TEXT NOT NULL, sender TEXT, member TEXT, body TEXT, ts INTEGER, attribution TEXT NOT NULL DEFAULT 'unattributed', label TEXT NOT NULL DEFAULT 'public');
 CREATE UNIQUE INDEX messages_room_seq ON messages(room, seq);
 CREATE UNIQUE INDEX messages_mid ON messages(mid);
 CREATE TABLE claims(id INTEGER PRIMARY KEY AUTOINCREMENT, tenant TEXT NOT NULL DEFAULT 'local', room TEXT, path TEXT, owner TEXT, member TEXT, reason TEXT, expires INTEGER);";

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
                    migrate_v1(&conn)?;
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

    fn members(&self, room: &str) -> Result<Vec<MemberInfo>> {
        let mut st = self
            .conn
            .prepare("SELECT name,last_seen FROM members WHERE room=?1 ORDER BY name")
            .map_err(be)?;
        st.query_map(params![room], |r| {
            Ok(MemberInfo {
                name: r.get(0)?,
                last_seen: r.get(1)?,
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
        self.immediate(|| {
            self.conn
                .execute("DELETE FROM claims WHERE expires<=?1", params![now])
                .map_err(be)?;
            let mut st = self
                .conn
                .prepare("SELECT path,owner,expires FROM claims WHERE room=?1")
                .map_err(be)?;
            let held = st
                .query_map(params![room], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                })
                .map_err(be)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(be)?;
            for (p, o, e) in held {
                if o != owner && overlaps(&p, path) {
                    return Err(Error::Conflict {
                        path: p,
                        owner: o,
                        secs_left: e - now,
                    });
                }
            }
            self.conn
                .execute(
                    "DELETE FROM claims WHERE room=?1 AND owner=?2 AND path=?3",
                    params![room, owner, path],
                )
                .map_err(be)?;
            self.conn
                .execute(
                    "INSERT INTO claims(room,path,owner,member,reason,expires) VALUES(?1,?2,?3,?3,?4,?5)",
                    params![room, path, owner, reason, now + ttl],
                )
                .map_err(be)?;
            Ok(())
        })
    }

    fn release(&self, room: &str, owner: &str, path: &str) -> Result<()> {
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
            .prepare("SELECT path,owner,reason,expires FROM claims WHERE room=?1 AND expires>?2 ORDER BY path")
            .map_err(be)?;
        st.query_map(params![room, now], |r| {
            Ok(Claim {
                path: r.get(0)?,
                owner: r.get(1)?,
                reason: r.get(2)?,
                expires: r.get(3)?,
            })
        })
        .map_err(be)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(be)
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
                last_seen: 20
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
                secs_left: 90
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
                expires: 200
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
    fn a_fresh_database_is_at_schema_version_2() {
        assert_eq!(version(&store()), 2);
    }

    #[test]
    fn a_version_1_database_is_migrated_to_2_and_keeps_its_messages() {
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
        assert_eq!(version(&s), 2);
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
            2,
            "reopening a migrated database is a no-op"
        );
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn a_version_0_database_now_migrates_all_the_way_to_2() {
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
        assert_eq!(version(&s), 2);
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

    #[test]
    fn a_database_written_by_the_real_prototype_migrates_with_everything_intact() {
        let path = temp_db("real-v0");
        std::fs::write(&path, PROTOTYPE_V0).unwrap();
        let s = SqliteStore::open(&path).unwrap();
        assert_eq!(version(&s), 2);
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
