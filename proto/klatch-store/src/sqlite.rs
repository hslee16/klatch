use crate::Store;
use klatch_core::{overlaps, Claim, Error, MemberInfo, Message, Result};
use rusqlite::{params, Connection};
use std::path::Path;
use std::time::Duration;

/// SQLite in WAL mode. Many readers, one writer at a time; a claim check-and-insert is one
/// `BEGIN IMMEDIATE` transaction so it is atomic across processes.
pub struct SqliteStore {
    conn: Connection,
}

fn be(e: rusqlite::Error) -> Error {
    Error::Backend(e.to_string())
}

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS members(room TEXT, name TEXT, joined INTEGER, last_seen INTEGER, last_id INTEGER DEFAULT 0, PRIMARY KEY(room,name));
 CREATE TABLE IF NOT EXISTS messages(id INTEGER PRIMARY KEY AUTOINCREMENT, room TEXT, sender TEXT, body TEXT, ts INTEGER);
 CREATE TABLE IF NOT EXISTS claims(id INTEGER PRIMARY KEY AUTOINCREMENT, room TEXT, path TEXT, owner TEXT, reason TEXT, expires INTEGER);";

impl SqliteStore {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).map_err(be)?;
        conn.busy_timeout(Duration::from_secs(5)).map_err(be)?;
        conn.pragma_update(None, "journal_mode", "WAL").map_err(be)?;
        conn.pragma_update(None, "synchronous", "NORMAL").map_err(be)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory().map_err(be)?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(SCHEMA).map_err(be)?;
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

    fn post(&self, room: &str, sender: &str, body: &str, now: i64) -> Result<i64> {
        self.conn
            .execute(
                "INSERT INTO messages(room,sender,body,ts) VALUES(?1,?2,?3,?4)",
                params![room, sender, body, now],
            )
            .map_err(be)?;
        let id = self.conn.last_insert_rowid();
        self.touch(room, sender, now)?;
        Ok(id)
    }

    fn read(&self, room: &str, member: &str, advance: bool, now: i64) -> Result<Vec<Message>> {
        self.touch(room, member, now)?;
        let last: i64 = self
            .conn
            .query_row("SELECT last_id FROM members WHERE room=?1 AND name=?2", params![room, member], |r| r.get(0))
            .unwrap_or(0);
        let mut st = self
            .conn
            .prepare("SELECT id,sender,body,ts FROM messages WHERE room=?1 AND id>?2 AND sender<>?3 ORDER BY id")
            .map_err(be)?;
        let rows = st
            .query_map(params![room, last, member], |r| {
                Ok(Message { id: r.get(0)?, sender: r.get(1)?, body: r.get(2)?, ts: r.get(3)? })
            })
            .map_err(be)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(be)?;
        if advance {
            let max: i64 = self
                .conn
                .query_row("SELECT COALESCE(MAX(id),0) FROM messages WHERE room=?1", params![room], |r| r.get(0))
                .map_err(be)?;
            self.conn
                .execute(
                    "UPDATE members SET last_id=?3, last_seen=?4 WHERE room=?1 AND name=?2",
                    params![room, member, max, now],
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
        st.query_map(params![room], |r| Ok(MemberInfo { name: r.get(0)?, last_seen: r.get(1)? }))
            .map_err(be)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(be)
    }

    fn claim(&self, room: &str, owner: &str, path: &str, ttl: i64, reason: &str, now: i64) -> Result<()> {
        self.immediate(|| {
            self.conn.execute("DELETE FROM claims WHERE expires<=?1", params![now]).map_err(be)?;
            let mut st = self
                .conn
                .prepare("SELECT path,owner,expires FROM claims WHERE room=?1")
                .map_err(be)?;
            let held = st
                .query_map(params![room], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))
                .map_err(be)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(be)?;
            for (p, o, e) in held {
                if o != owner && overlaps(&p, path) {
                    return Err(Error::Conflict { path: p, owner: o, secs_left: e - now });
                }
            }
            self.conn
                .execute("DELETE FROM claims WHERE room=?1 AND owner=?2 AND path=?3", params![room, owner, path])
                .map_err(be)?;
            self.conn
                .execute(
                    "INSERT INTO claims(room,path,owner,reason,expires) VALUES(?1,?2,?3,?4,?5)",
                    params![room, path, owner, reason, now + ttl],
                )
                .map_err(be)?;
            Ok(())
        })
    }

    fn release(&self, room: &str, owner: &str, path: &str) -> Result<()> {
        let n = self
            .conn
            .execute("DELETE FROM claims WHERE room=?1 AND path=?2 AND owner=?3", params![room, path, owner])
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
            Ok(Claim { path: r.get(0)?, owner: r.get(1)?, reason: r.get(2)?, expires: r.get(3)? })
        })
        .map_err(be)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(be)
    }

    fn rooms_of(&self, member: &str) -> Result<Vec<String>> {
        let mut st = self.conn.prepare("SELECT room FROM members WHERE name=?1 ORDER BY room").map_err(be)?;
        st.query_map(params![member], |r| r.get(0))
            .map_err(be)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(be)
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
        assert_eq!(m, vec![MemberInfo { name: "alice".into(), last_seen: 20 }]);
        assert_eq!(s.rooms_of("alice").unwrap(), vec!["r".to_string()]);
    }

    #[test]
    fn rooms_of_is_sorted_and_per_member() {
        let s = store();
        s.join("b", "alice", 1).unwrap();
        s.join("a", "alice", 1).unwrap();
        s.join("c", "bob", 1).unwrap();
        assert_eq!(s.rooms_of("alice").unwrap(), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn post_returns_increasing_ids_and_marks_sender_present() {
        let s = store();
        let a = s.post("r", "alice", "one", 5).unwrap();
        let b = s.post("r", "alice", "two", 6).unwrap();
        assert!(b > a);
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
        assert_eq!((first[0].sender.as_str(), first[0].body.as_str()), ("bob", "hi"));
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
        assert_eq!(err, Error::Conflict { path: "src".into(), owner: "bob".into(), secs_left: 90 });
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
        assert_eq!(c, vec![Claim { path: "src".into(), owner: "bob".into(), reason: "b".into(), expires: 200 }]);
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
        let paths: Vec<_> = s.claims("r", 1).unwrap().into_iter().map(|c| c.path).collect();
        assert_eq!(paths, vec!["a", "z"]);
    }

    #[test]
    fn release_removes_only_your_exact_claim() {
        let s = store();
        s.claim("r", "bob", "src", 100, "", 0).unwrap();
        assert!(matches!(s.release("r", "alice", "src"), Err(Error::NotFound(_))));
        assert!(matches!(s.release("r", "bob", "other"), Err(Error::NotFound(_))));
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
}
