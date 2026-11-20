//! Re-keying the archive files when the store key rotates (#628): every archive moves to the new key before the old
//! key is dropped, so archived messages can still be restored afterwards.

use super::*;
use crate::domain::Error;
use crate::keys::Key;
use crate::store::sqlite::archive::{
    ArchiveState, list_archives, rekey_archives, rekey_archives_hooked, rekey_archives_in,
};
use std::path::{Path, PathBuf};

fn archive_dir(tag: &str) -> crate::store::test_dir::TestDir {
    crate::store::test_dir::TestDir::new(&format!("rekey-{tag}"))
}

/// An archive of three old messages of room `r`, made with the test key.
fn one_archive(dir: &Path, name: &str) -> PathBuf {
    let s = store();
    s.join("r", "alice", 1).unwrap();
    for (i, body) in ["old one", "old two", "old three"].iter().enumerate() {
        s.post(Seat::new("r", "alice"), body, 10 + i as i64)
            .unwrap();
    }
    let out = dir.join(name);
    s.archive_before("r", 500, &out, &crate::store::insecure_test_key())
        .unwrap();
    out
}

fn other_key() -> Key {
    Key::generate().unwrap()
}

fn opens_with(file: &Path, key: &Key) -> bool {
    crate::store::encrypt::check_opens(file, key).is_ok()
}

#[test]
fn rekeying_moves_every_archive_to_the_new_key_and_keeps_its_messages() {
    let dir = archive_dir("all");
    let (a, b) = (one_archive(&dir, "r-100.db"), one_archive(&dir, "r-200.db"));
    let (old, new) = (crate::store::insecure_test_key(), other_key());
    assert_eq!(rekey_archives(&dir, &old, &new).unwrap(), 2);
    for f in [&a, &b] {
        assert!(opens_with(f, &new), "{f:?} does not open with the new key");
        assert!(!opens_with(f, &old), "{f:?} still opens with the old key");
    }
    let found = list_archives(&dir, &new).unwrap();
    assert!(
        found
            .iter()
            .all(|f| matches!(f.state, ArchiveState::Ok { messages: 3, .. }))
    );
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        2,
        "a temp file was left behind"
    );
}

#[test]
fn a_rekeyed_archive_still_restores_into_the_store() {
    let dir = archive_dir("restore");
    let s = store();
    s.join("r", "alice", 1).unwrap();
    s.post(Seat::new("r", "alice"), "old one", 10).unwrap();
    let (old, new) = (crate::store::insecure_test_key(), other_key());
    let out = dir.join("r-100.db");
    s.archive_before("r", 500, &out, &old).unwrap();
    rekey_archives(&dir, &old, &new).unwrap();
    assert_eq!(s.restore_archive(&out, &new).unwrap(), 1);
    assert_eq!(s.tail("r", 0, 10).unwrap()[0].body, "old one");
}

#[test]
fn an_archive_that_opens_with_neither_key_stops_the_rekey_before_anything_changes() {
    let dir = archive_dir("bad");
    let good = one_archive(&dir, "r-100.db");
    let bad = dir.join("r-200.db");
    std::fs::copy(&good, &bad).unwrap();
    let old = crate::store::insecure_test_key();
    // unreadable to both keys: moved to a third one
    rekey_archives_in(&bad, &old, &other_key()).unwrap();
    let new = other_key();
    let err = rekey_archives(&dir, &old, &new).unwrap_err().to_string();
    assert!(err.contains("r-200.db"), "{err}");
    assert!(opens_with(&good, &old), "the good file was changed");
    assert!(!opens_with(&good, &new), "the good file was changed");
}

#[test]
fn a_rekey_that_died_half_way_leaves_every_file_under_one_of_the_two_keys_and_can_be_repeated() {
    let dir = archive_dir("resume");
    let files: Vec<_> = ["r-100.db", "r-200.db", "r-300.db"]
        .iter()
        .map(|n| one_archive(&dir, n))
        .collect();
    let (old, new) = (crate::store::insecure_test_key(), other_key());
    let died = rekey_archives_hooked(&dir, &old, &new, &|done| {
        if done == 1 {
            Err(Error::Backend("killed".into()))
        } else {
            Ok(())
        }
    });
    assert!(died.is_err());
    for f in &files {
        assert!(
            opens_with(f, &old) || opens_with(f, &new),
            "{f:?} opens with neither key after the interruption"
        );
    }
    // the second run skips what is done and finishes the rest
    assert_eq!(rekey_archives(&dir, &old, &new).unwrap(), 2);
    assert!(files.iter().all(|f| opens_with(f, &new)));
}

#[test]
fn a_partial_file_and_files_that_are_not_archives_are_left_alone_by_a_rekey() {
    let dir = archive_dir("other");
    one_archive(&dir, "r-100.db");
    std::fs::write(dir.join("r-100.db.partial"), "half").unwrap();
    std::fs::write(dir.join("notes.txt"), "mine").unwrap();
    let (old, new) = (crate::store::insecure_test_key(), other_key());
    assert_eq!(rekey_archives(&dir, &old, &new).unwrap(), 1);
    assert_eq!(std::fs::read(dir.join("notes.txt")).unwrap(), b"mine");
    assert_eq!(
        std::fs::read(dir.join("r-100.db.partial")).unwrap(),
        b"half"
    );
}

#[test]
fn a_missing_archive_directory_has_nothing_to_rekey() {
    let dir = archive_dir("none").join("absent");
    let (old, new) = (crate::store::insecure_test_key(), other_key());
    assert_eq!(rekey_archives(&dir, &old, &new).unwrap(), 0);
}
