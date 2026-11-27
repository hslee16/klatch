//! The read-only helpers `klatch doctor` uses (#1584): they never create or write a store, and a store with no policy
//! table is "not set", not an error.

use crate::keys::Key;
use crate::store::SqliteStore;

fn key() -> Key {
    Key::from_bytes([7u8; crate::keys::KEY_LEN])
}

#[test]
fn reading_a_missing_store_fails_and_creates_no_file() {
    let d = crate::store::test_dir::TestDir::new("probe-missing");
    let db = d.join("rooms.db");
    assert!(SqliteStore::read_policy_has(&db, &key(), "k").is_err());
    assert!(SqliteStore::read_growth(&db, &key()).is_err());
    assert!(SqliteStore::read_member_modes(&db, &key()).is_err());
    assert!(!db.exists(), "a read-only open must not create the file");
}

#[test]
fn a_store_without_a_policy_table_has_no_policy_key() {
    let d = crate::store::test_dir::TestDir::new("probe-nopolicy");
    let db = d.join("rooms.db");
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.pragma_update(None, "key", key().sqlcipher_pragma().as_str())
            .unwrap();
        conn.execute_batch("CREATE TABLE other(x)").unwrap();
    }
    assert_eq!(
        SqliteStore::read_policy_has(&db, &key(), "k").ok(),
        Some(false)
    );
}

#[test]
fn a_wrong_key_is_an_error_not_an_answer() {
    let d = crate::store::test_dir::TestDir::new("probe-wrongkey");
    let db = d.join("rooms.db");
    drop(SqliteStore::open(&db, &key()).unwrap());
    let wrong = Key::from_bytes([9u8; crate::keys::KEY_LEN]);
    assert!(SqliteStore::read_policy_has(&db, &wrong, "k").is_err());
}

#[test]
fn a_read_only_open_cannot_write() {
    let d = crate::store::test_dir::TestDir::new("probe-ro");
    let db = d.join("rooms.db");
    drop(SqliteStore::open(&db, &key()).unwrap());
    let before = std::fs::read(&db).unwrap();
    assert_eq!(
        SqliteStore::read_policy_has(&db, &key(), "admin-passphrase").ok(),
        Some(false)
    );
    assert_eq!(std::fs::read(&db).unwrap(), before);
}

/// The probe connection cannot write, whatever the platform's flag values are (#1709).
#[test]
fn the_probe_connection_is_read_only_and_refuses_a_write() {
    let d = crate::store::test_dir::TestDir::new("probe-flags");
    let db = d.join("rooms.db");
    drop(SqliteStore::open(&db, &key()).unwrap());
    let conn = SqliteStore::open_probe(&db).unwrap();
    SqliteStore::unlock(&conn, &key()).unwrap();
    assert!(conn.is_readonly("main").unwrap());
    let e = conn.execute_batch("CREATE TABLE intruder(x)").unwrap_err();
    assert!(e.to_string().contains("readonly"), "{e}");
}

/// A writer holding a rollback-journal store for a moment is waited for, not reported as a failure (#1701, #1709):
/// without the busy timeout the probe fails at once.
#[test]
fn a_probe_waits_for_a_writer_that_holds_the_store_briefly() {
    let d = crate::store::test_dir::TestDir::new("probe-busy");
    let db = d.join("rooms.db");
    drop(SqliteStore::open(&db, &key()).unwrap());
    let writer = rusqlite::Connection::open(&db).unwrap();
    writer
        .pragma_update(None, "key", key().sqlcipher_pragma().as_str())
        .unwrap();
    writer
        .pragma_update(None, "journal_mode", "DELETE")
        .unwrap();
    writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        writer.execute_batch("COMMIT").unwrap();
        writer
    });
    let start = std::time::Instant::now();
    let seen = SqliteStore::read_policy_has(&db, &key(), "k");
    assert_eq!(
        seen.ok(),
        Some(false),
        "the probe must wait out a short lock"
    );
    assert!(start.elapsed() >= std::time::Duration::from_millis(100));
    drop(release.join().unwrap());
}

/// A writer that never lets go is an error after the wait, not a hang.
#[test]
fn a_probe_gives_up_on_a_writer_that_never_lets_go() {
    let d = crate::store::test_dir::TestDir::new("probe-stuck");
    let db = d.join("rooms.db");
    drop(SqliteStore::open(&db, &key()).unwrap());
    let writer = rusqlite::Connection::open(&db).unwrap();
    writer
        .pragma_update(None, "key", key().sqlcipher_pragma().as_str())
        .unwrap();
    writer
        .pragma_update(None, "journal_mode", "DELETE")
        .unwrap();
    writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let start = std::time::Instant::now();
    assert!(SqliteStore::read_policy_has(&db, &key(), "k").is_err());
    assert!(start.elapsed() >= std::time::Duration::from_millis(500));
    assert!(start.elapsed() < std::time::Duration::from_secs(30));
}

/// What a read-only open leaves beside a WAL store, stated exactly (#1701): the main file is unchanged and the only
/// files that may appear are SQLite's own `-wal` and `-shm`; nothing else is created. A store closed cleanly has
/// neither, so a probe that left a `-wal` with content, or any other file, is caught.
#[test]
fn a_read_only_open_leaves_only_the_main_file_and_sqlites_wal_and_shm() {
    let d = crate::store::test_dir::TestDir::new("probe-ro-files");
    let db = d.join("rooms.db");
    drop(SqliteStore::open(&db, &key()).unwrap());
    let beside = |d: &std::path::Path| -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    };
    assert_eq!(
        beside(&d),
        ["rooms.db"],
        "a closed store has no -wal or -shm to begin with"
    );
    let before = std::fs::read(&db).unwrap();
    let mtime = std::fs::metadata(&db).unwrap().modified().unwrap();
    SqliteStore::read_policy_has(&db, &key(), "k").unwrap();
    SqliteStore::read_growth(&db, &key()).unwrap();
    SqliteStore::read_member_modes(&db, &key()).unwrap();
    assert_eq!(std::fs::read(&db).unwrap(), before);
    assert_eq!(std::fs::metadata(&db).unwrap().modified().unwrap(), mtime);
    let mut names = beside(&d);
    names.retain(|n| !["rooms.db", "rooms.db-wal", "rooms.db-shm"].contains(&n.as_str()));
    assert!(names.is_empty(), "unexpected files: {names:?}");
    // whatever -wal exists holds no frames: a probe wrote nothing to the log
    if let Ok(m) = std::fs::metadata(d.join("rooms.db-wal")) {
        assert_eq!(m.len(), 0, "the probe left frames in the -wal");
    }
}
