//! Listing and checking the archive files under the klatch directory (#487): each is opened with the store key and
//! passes SQLCipher's page check, or is reported with the reason; nothing is skipped silently and nothing is changed.

use super::*;
use crate::store::sqlite::archive::{ArchiveState, list_archives};
use std::path::{Path, PathBuf};

fn archive_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("klatch-archives-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// An archive of three old messages (times 10, 20, 30) of room `r`, made with the test key.
fn one_archive(dir: &Path, name: &str) -> PathBuf {
    let s = store();
    s.join("r", "alice", 1).unwrap();
    s.post(Seat::new("r", "alice"), "old one", 10).unwrap();
    s.post(Seat::new("r", "alice"), "old two", 20).unwrap();
    s.post(Seat::new("r", "alice"), "old three", 30).unwrap();
    s.post(Seat::new("r", "alice"), "new one", 1000).unwrap();
    let out = dir.join(name);
    s.archive_before("r", 500, &out, &crate::store::insecure_test_key())
        .unwrap();
    out
}

#[test]
fn an_archive_is_listed_with_its_rooms_count_and_age_range() {
    let dir = archive_dir("list");
    let file = one_archive(&dir, "r-100.db");
    let found = list_archives(&dir, &crate::store::insecure_test_key()).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, file);
    assert!(found[0].bytes > 0);
    assert_eq!(
        found[0].state,
        ArchiveState::Ok {
            rooms: vec!["r".to_string()],
            messages: 3,
            oldest: 10,
            newest: 30
        }
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_missing_directory_has_no_archives() {
    let dir = archive_dir("none").join("absent");
    assert!(
        list_archives(&dir, &crate::store::insecure_test_key())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn listing_is_in_name_order_and_ignores_files_that_are_not_archives() {
    let dir = archive_dir("order");
    one_archive(&dir, "r-200.db");
    one_archive(&dir, "r-100.db");
    std::fs::write(dir.join("notes.txt"), "mine").unwrap();
    let found = list_archives(&dir, &crate::store::insecure_test_key()).unwrap();
    let names: Vec<_> = found
        .iter()
        .map(|a| a.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["r-100.db", "r-200.db"]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_archive_that_does_not_open_with_the_store_key_is_reported_not_skipped() {
    let dir = archive_dir("wrongkey");
    one_archive(&dir, "r-100.db");
    let other = crate::keys::Key::from_bytes([7u8; crate::keys::KEY_LEN]);
    let found = list_archives(&dir, &other).unwrap();
    assert!(
        matches!(&found[0].state, ArchiveState::Unreadable(why) if why.contains("key")),
        "{:?}",
        found[0].state
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_tampered_archive_is_reported_damaged() {
    let dir = archive_dir("tamper");
    let file = one_archive(&dir, "r-100.db");
    let mut bytes = std::fs::read(&file).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0xff;
    std::fs::write(&file, bytes).unwrap();
    let found = list_archives(&dir, &crate::store::insecure_test_key()).unwrap();
    assert!(
        matches!(found[0].state, ArchiveState::Unreadable(_)),
        "{:?}",
        found[0].state
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_plaintext_database_among_the_archives_is_reported_as_unencrypted() {
    let dir = archive_dir("plain");
    let c = rusqlite::Connection::open(dir.join("old-1.db")).unwrap();
    c.execute_batch("CREATE TABLE archived(room TEXT); INSERT INTO archived VALUES ('r');")
        .unwrap();
    drop(c);
    let found = list_archives(&dir, &crate::store::insecure_test_key()).unwrap();
    assert!(
        matches!(&found[0].state, ArchiveState::Unreadable(why) if why.contains("not encrypted")),
        "{:?}",
        found[0].state
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_partial_file_left_by_an_interrupted_archive_is_reported() {
    let dir = archive_dir("partial");
    std::fs::write(dir.join("r-100.db.partial"), "half").unwrap();
    let found = list_archives(&dir, &crate::store::insecure_test_key()).unwrap();
    assert_eq!(found[0].state, ArchiveState::Partial);
    let _ = std::fs::remove_dir_all(dir);
}
