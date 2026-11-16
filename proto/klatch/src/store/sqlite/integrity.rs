//! A read-only check of a whole store, for the bench (#621): the file passes SQLite's own check, every room's `seq`
//! runs 1..N with no gap, and every tombstone has its message in an archive file. It opens the files read-only, so
//! looking changes nothing (no migration, no journal switch), and it reports counts and room names, never message
//! text.

use crate::domain::{Error, Result};
use crate::keys::Key;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;

fn be(e: rusqlite::Error) -> Error {
    Error::Backend(e.to_string())
}

/// What SQLite's `PRAGMA integrity_check` says, as problems (it says `ok` once when all is well).
fn page_check(conn: &Connection) -> Result<Vec<String>> {
    let mut st = conn.prepare("PRAGMA integrity_check").map_err(be)?;
    let lines = st
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(be)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(be)?;
    Ok(lines
        .into_iter()
        .filter(|l| l != "ok")
        .map(|l| format!("integrity_check: {l}"))
        .collect())
}

/// Rooms whose `seq` is not exactly 1..N: a gap, a repeat, or a start other than 1. Tombstones keep their seq, so a
/// healthy store, archived or not, has none.
fn seq_density(conn: &Connection) -> Result<Vec<String>> {
    let mut st = conn
        .prepare(
            "SELECT room, count(*), min(seq), max(seq) FROM messages GROUP BY room ORDER BY room",
        )
        .map_err(be)?;
    let rooms = st
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })
        .map_err(be)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(be)?;
    Ok(rooms
        .into_iter()
        .filter(|(_, n, lo, hi)| *lo != 1 || hi != n)
        .map(|(room, n, lo, hi)| {
            format!("seq of room {room}: {n} messages but seq runs {lo}..{hi}")
        })
        .collect())
}

/// Tombstones (rows whose text moved to an archive) with no matching message in any of `archives`.
fn tombstones(conn: &Connection, archives: &[Connection]) -> Result<Vec<String>> {
    let mut st = conn
        .prepare("SELECT room, seq, mid FROM messages WHERE kind='archived' ORDER BY room, seq")
        .map_err(be)?;
    let rows = st
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(be)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(be)?;
    let mut problems = Vec::new();
    for (room, seq, mid) in rows {
        if !archived_somewhere(archives, &room, seq, &mid)? {
            problems.push(format!(
                "tombstone {room}#{seq} ({mid}) has no message in any archive file"
            ));
        }
    }
    Ok(problems)
}

fn archived_somewhere(archives: &[Connection], room: &str, seq: i64, mid: &str) -> Result<bool> {
    for a in archives {
        let n: i64 = a
            .query_row(
                "SELECT count(*) FROM archived WHERE room=?1 AND seq=?2 AND mid=?3",
                rusqlite::params![room, seq, mid],
                |r| r.get(0),
            )
            .map_err(be)?;
        if n > 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

fn open_read_only(path: &Path, key: &Key) -> Result<Connection> {
    let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(be)?;
    c.pragma_update(None, "key", key.sqlcipher_pragma().as_str())
        .map_err(be)?;
    c.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })
    .map_err(|e| Error::Backend(format!("{}: {e}", path.display())))?;
    Ok(c)
}

/// The archive files next to the store (`archive/*.db`), opened read-only; a missing directory has none.
fn open_archives(dir: &Path, key: &Key) -> Result<Vec<Connection>> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    let mut paths: Vec<_> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "db"))
        .collect();
    paths.sort();
    paths.iter().map(|p| open_read_only(p, key)).collect()
}

/// Check the store at `db` (archives in `archive/` beside it) with `key`: every problem found, none when it is
/// sound. An error means the store could not be examined at all (it would not open with the key).
pub fn check_store(db: &Path, key: &Key) -> Result<Vec<String>> {
    let conn = open_read_only(db, key)?;
    let archives = open_archives(&db.with_file_name("archive"), key)?;
    let mut problems = page_check(&conn)?;
    problems.extend(seq_density(&conn)?);
    problems.extend(tombstones(&conn, &archives)?);
    Ok(problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE messages(room TEXT, seq INTEGER, mid TEXT, kind TEXT);
             CREATE UNIQUE INDEX m ON messages(room, seq);",
        )
        .unwrap();
        c
    }

    fn archive(rows: &[(&str, i64, &str)]) -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE archived(room TEXT, seq INTEGER, mid TEXT);")
            .unwrap();
        for (r, s, m) in rows {
            c.execute(
                "INSERT INTO archived VALUES(?1,?2,?3)",
                rusqlite::params![r, s, m],
            )
            .unwrap();
        }
        c
    }

    fn put(c: &Connection, room: &str, seq: i64, mid: &str, kind: Option<&str>) {
        c.execute(
            "INSERT INTO messages VALUES(?1,?2,?3,?4)",
            rusqlite::params![room, seq, mid, kind],
        )
        .unwrap();
    }

    #[test]
    fn a_dense_store_has_no_seq_problem() {
        let c = store();
        for s in 1..=3 {
            put(&c, "a", s, &format!("a{s}"), None);
        }
        put(&c, "b", 1, "b1", None);
        assert_eq!(seq_density(&c).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn a_gap_in_a_rooms_seq_is_reported_with_the_room() {
        let c = store();
        put(&c, "a", 1, "a1", None);
        put(&c, "a", 3, "a3", None);
        let p = seq_density(&c).unwrap();
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].contains("room a"), "{p:?}");
    }

    #[test]
    fn a_room_that_does_not_start_at_one_is_reported() {
        let c = store();
        put(&c, "a", 2, "a2", None);
        assert_eq!(seq_density(&c).unwrap().len(), 1);
    }

    #[test]
    fn a_tombstone_with_its_archived_message_is_consistent() {
        let c = store();
        put(&c, "a", 1, "m1", Some("archived"));
        put(&c, "a", 2, "m2", None);
        let ar = archive(&[("a", 1, "m1")]);
        assert_eq!(tombstones(&c, &[ar]).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn a_tombstone_whose_message_is_in_no_archive_is_reported() {
        let c = store();
        put(&c, "a", 1, "m1", Some("archived"));
        let other = archive(&[("a", 1, "different-mid")]);
        let p = tombstones(&c, &[other]).unwrap();
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].contains("a#1"), "{p:?}");
        assert_eq!(tombstones(&c, &[]).unwrap().len(), 1);
    }

    #[test]
    fn a_damaged_file_fails_the_page_check() {
        let dir = std::env::temp_dir().join(format!("klatch-integrity-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.db");
        let _ = std::fs::remove_file(&path);
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch("PRAGMA journal_mode=DELETE; CREATE TABLE t(a TEXT, b INTEGER); CREATE INDEX ti ON t(a);")
                .unwrap();
            for i in 0..2000 {
                c.execute(
                    "INSERT INTO t VALUES(?1,?2)",
                    rusqlite::params![format!("v{i:05}"), i],
                )
                .unwrap();
            }
        }
        let c = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert_eq!(page_check(&c).unwrap(), Vec::<String>::new());
        drop(c);
        let mut bytes = std::fs::read(&path).unwrap();
        let n = bytes.len();
        for b in &mut bytes[n / 2..n / 2 + 600] {
            *b = 0xFF;
        }
        std::fs::write(&path, bytes).unwrap();
        let c = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let p = page_check(&c).unwrap();
        assert!(!p.is_empty(), "a damaged file passed the page check");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
