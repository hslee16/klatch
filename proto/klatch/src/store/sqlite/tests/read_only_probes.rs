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

/// What a read-only open leaves beside a WAL store, stated exactly (#1701): the main file is unchanged and the only
/// files that may appear are SQLite's own `-wal` and `-shm`; nothing else is created.
#[test]
fn a_read_only_open_leaves_only_the_main_file_and_sqlites_wal_and_shm() {
    let d = crate::store::test_dir::TestDir::new("probe-ro-files");
    let db = d.join("rooms.db");
    drop(SqliteStore::open(&db, &key()).unwrap());
    let before = std::fs::read(&db).unwrap();
    let mtime = std::fs::metadata(&db).unwrap().modified().unwrap();
    SqliteStore::read_policy_has(&db, &key(), "k").unwrap();
    SqliteStore::read_growth(&db, &key()).unwrap();
    SqliteStore::read_member_modes(&db, &key()).unwrap();
    assert_eq!(std::fs::read(&db).unwrap(), before);
    assert_eq!(std::fs::metadata(&db).unwrap().modified().unwrap(), mtime);
    let mut names: Vec<String> = std::fs::read_dir(&*d)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.retain(|n| !["rooms.db", "rooms.db-wal", "rooms.db-shm"].contains(&n.as_str()));
    assert!(names.is_empty(), "unexpected files: {names:?}");
}
