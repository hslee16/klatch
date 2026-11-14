//! Moving a plaintext database to the encrypted format (#164, R39): in place, verified before anything is removed,
//! resumable after a crash, and never leaving a readable copy behind.

use super::*;
use crate::encrypt::{Outcome, ensure_encrypted, scrub_and_remove};

const PROTOTYPE_V0: &[u8] = include_bytes!("../../../fixtures/prototype-v0.db");
const RELEASE_V2: &[u8] = include_bytes!("../../../fixtures/prototype-v2.db");

fn key() -> klatch_keys::Key {
    crate::insecure_test_key()
}

fn names(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn has(bytes: &[u8], needle: &str) -> bool {
    bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
}

#[test]
fn a_release_database_is_converted_and_its_data_survives() {
    let path = temp_db("pt-v2");
    std::fs::write(&path, RELEASE_V2).unwrap();
    assert!(has(&std::fs::read(&path).unwrap(), "hello from v2"));
    let outcome = ensure_encrypted(&path, &key()).unwrap();
    assert!(matches!(outcome, Outcome::Encrypted { .. }), "{outcome:?}");
    assert!(
        !has(&std::fs::read(&path).unwrap(), "hello from v2"),
        "still readable"
    );
    let s = SqliteStore::open(&path, &key()).unwrap();
    assert_eq!(s.tail("core", 0, 10).unwrap()[0].body, "hello from v2");
    assert_eq!(s.claims("core", 1).unwrap().len(), 1);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn the_oldest_prototype_database_is_converted_and_then_schema_migrated() {
    let path = temp_db("pt-v0");
    std::fs::write(&path, PROTOTYPE_V0).unwrap();
    ensure_encrypted(&path, &key()).unwrap();
    let s = SqliteStore::open(&path, &key()).unwrap();
    let core = s.tail("core", 0, 10).unwrap();
    assert_eq!(
        core.iter().map(|m| m.body.as_str()).collect::<Vec<_>>(),
        vec!["alpha", "beta", "gamma"]
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn nothing_but_the_database_is_left_behind() {
    let path = temp_db("pt-clean");
    std::fs::write(&path, RELEASE_V2).unwrap();
    ensure_encrypted(&path, &key()).unwrap();
    assert_eq!(names(path.parent().unwrap()), vec!["rooms.db".to_string()]);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn an_encrypted_or_missing_database_is_left_alone_and_a_second_run_changes_nothing() {
    let path = temp_db("pt-twice");
    assert_eq!(ensure_encrypted(&path, &key()).unwrap(), Outcome::Nothing);
    assert!(!path.exists(), "a missing database was created");
    std::fs::write(&path, RELEASE_V2).unwrap();
    ensure_encrypted(&path, &key()).unwrap();
    let after_first = std::fs::read(&path).unwrap();
    assert_eq!(ensure_encrypted(&path, &key()).unwrap(), Outcome::Nothing);
    assert_eq!(std::fs::read(&path).unwrap(), after_first);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_database_with_a_write_ahead_log_keeps_what_the_log_held() {
    let path = temp_db("pt-wal");
    {
        // a plaintext database whose latest rows live only in the WAL
        let c = Connection::open(&path).unwrap();
        c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
            .unwrap();
        c.execute_batch(
            "CREATE TABLE messages(id INTEGER PRIMARY KEY, room TEXT, body TEXT); INSERT INTO messages(room,body) VALUES('r','from-the-wal-xyz');",
        )
        .unwrap();
        std::mem::forget(c); // leave the log in place, as a crash would
    }
    assert!(
        std::fs::metadata(format!("{}-wal", path.display()))
            .map(|m| m.len() > 0)
            .unwrap_or(false)
    );
    ensure_encrypted(&path, &key()).unwrap();
    for f in names(path.parent().unwrap()) {
        let bytes = std::fs::read(path.parent().unwrap().join(&f)).unwrap();
        assert!(!has(&bytes, "from-the-wal-xyz"), "{f} still has it");
    }
    let c = keyed(&path);
    let body: String = c
        .query_row("SELECT body FROM messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(body, "from-the-wal-xyz");
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_crash_while_copying_leaves_the_original_and_the_next_run_finishes_the_job() {
    let path = temp_db("pt-crash-copy");
    std::fs::write(&path, RELEASE_V2).unwrap();
    // the half-written encrypted copy a killed run leaves
    let partial = format!("{}.encrypting", path.display());
    std::fs::write(&partial, vec![0xEE; 5000]).unwrap();
    ensure_encrypted(&path, &key()).unwrap();
    assert_eq!(names(path.parent().unwrap()), vec!["rooms.db".to_string()]);
    assert_eq!(
        SqliteStore::open(&path, &key())
            .unwrap()
            .tail("core", 0, 10)
            .unwrap()
            .len(),
        1
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_crash_after_the_swap_leaves_a_plaintext_link_that_the_next_run_scrubs() {
    let path = temp_db("pt-crash-swap");
    std::fs::write(&path, RELEASE_V2).unwrap();
    ensure_encrypted(&path, &key()).unwrap();
    // what a run killed between the swap and the scrub leaves: the old plaintext under a second name
    let old = format!("{}.old", path.display());
    std::fs::write(&old, RELEASE_V2).unwrap();
    assert_eq!(ensure_encrypted(&path, &key()).unwrap(), Outcome::Scrubbed);
    assert!(!std::path::Path::new(&old).exists());
    assert_eq!(
        SqliteStore::open(&path, &key())
            .unwrap()
            .tail("core", 0, 10)
            .unwrap()
            .len(),
        1
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn scrubbing_overwrites_every_byte_before_removing_the_file() {
    let dir = temp_db("pt-scrub");
    let dir = dir.parent().unwrap().to_path_buf();
    let (a, b) = (dir.join("a"), dir.join("b"));
    std::fs::write(&a, b"top secret plaintext").unwrap();
    std::fs::hard_link(&a, &b).unwrap(); // a second name keeps the content reachable after the first is removed
    scrub_and_remove(&a).unwrap();
    assert!(!a.exists());
    let left = std::fs::read(&b).unwrap();
    assert_eq!(left.len(), "top secret plaintext".len());
    assert!(
        left.iter().all(|x| *x == 0),
        "the content was not overwritten"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn several_processes_converting_at_once_convert_once_and_all_succeed() {
    let path = temp_db("pt-race");
    std::fs::write(&path, RELEASE_V2).unwrap();
    let handles: Vec<_> = (0..6)
        .map(|_| {
            let p = path.clone();
            std::thread::spawn(move || ensure_encrypted(&p, &crate::insecure_test_key()))
        })
        .collect();
    let outcomes: Vec<Outcome> = handles
        .into_iter()
        .map(|h| h.join().unwrap().unwrap())
        .collect();
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| matches!(o, Outcome::Encrypted { .. }))
            .count(),
        1,
        "{outcomes:?}"
    );
    assert_eq!(names(path.parent().unwrap()), vec!["rooms.db".to_string()]);
    assert_eq!(
        SqliteStore::open(&path, &key())
            .unwrap()
            .tail("core", 0, 10)
            .unwrap()
            .len(),
        1
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_file_that_is_not_a_database_is_refused_and_left_untouched() {
    let path = temp_db("pt-junk");
    std::fs::write(
        &path,
        b"this is neither sqlite nor ciphertext, long enough to be a file",
    )
    .unwrap();
    // not the plaintext header: treated as already encrypted, so nothing is done to it
    assert_eq!(ensure_encrypted(&path, &key()).unwrap(), Outcome::Nothing);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"this is neither sqlite nor ciphertext, long enough to be a file"
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

// ---- backups: an archive is encrypted like the store (#159) ----

#[test]
fn a_backup_is_an_encrypted_copy_with_no_plaintext_and_opens_with_the_same_key() {
    let path = temp_db("bk-copy");
    let s = SqliteStore::open(&path, &key()).unwrap();
    s.join("vault-room", "member-q", 1).unwrap();
    s.post(Seat::new("vault-room", "member-q"), "backup-secret-body", 2)
        .unwrap();
    let out = path.with_file_name("backup.db");
    s.backup_to(&out, &key()).unwrap();
    let bytes = std::fs::read(&out).unwrap();
    for secret in ["backup-secret-body", "vault-room", "member-q"] {
        assert!(!has(&bytes, secret), "{secret} is readable in the backup");
    }
    assert!(!bytes.starts_with(b"SQLite format 3"));
    let copy = SqliteStore::open(&out, &key()).unwrap();
    assert_eq!(
        copy.tail("vault-room", 0, 10).unwrap()[0].body,
        "backup-secret-body"
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_backup_never_overwrites_a_file_and_leaves_nothing_half_written() {
    let path = temp_db("bk-noover");
    let s = SqliteStore::open(&path, &key()).unwrap();
    let out = path.with_file_name("backup.db");
    std::fs::write(&out, b"precious").unwrap();
    assert!(s.backup_to(&out, &key()).is_err());
    assert_eq!(std::fs::read(&out).unwrap(), b"precious");
    let names = names(path.parent().unwrap());
    assert!(names.iter().all(|n| !n.ends_with(".partial")), "{names:?}");
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn restoring_a_backup_replaces_the_store_and_keeps_the_old_one_aside() {
    let path = temp_db("bk-restore");
    let s = SqliteStore::open(&path, &key()).unwrap();
    s.join("r", "alice", 1).unwrap();
    s.post(Seat::new("r", "alice"), "before", 2).unwrap();
    let out = path.with_file_name("backup.db");
    s.backup_to(&out, &key()).unwrap();
    s.post(Seat::new("r", "alice"), "after the backup", 3)
        .unwrap();
    drop(s);
    crate::encrypt::restore_backup(&path, &out, &key()).unwrap();
    let s = SqliteStore::open(&path, &key()).unwrap();
    let bodies: Vec<String> = s
        .tail("r", 0, 10)
        .unwrap()
        .into_iter()
        .map(|m| m.body)
        .collect();
    assert_eq!(bodies, vec!["before".to_string()]);
    // the store it replaced is kept, still encrypted, so a mistaken restore can be undone
    let aside = path.with_file_name("rooms.db.before-restore");
    assert!(aside.exists());
    assert!(!has(&std::fs::read(&aside).unwrap(), "after the backup"));
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_backup_made_under_another_key_or_damaged_is_refused_and_changes_nothing() {
    let path = temp_db("bk-bad");
    let s = SqliteStore::open(&path, &key()).unwrap();
    s.join("r", "alice", 1).unwrap();
    let other = klatch_keys::Key::from_bytes([3; 32]);
    let foreign = path.with_file_name("foreign.db");
    SqliteStore::open(&foreign, &other)
        .unwrap()
        .join("x", "bob", 1)
        .unwrap();
    drop(s);
    let before = std::fs::read(&path).unwrap();
    assert!(crate::encrypt::restore_backup(&path, &foreign, &key()).is_err());
    let junk = path.with_file_name("junk.db");
    std::fs::write(&junk, vec![7u8; 9000]).unwrap();
    assert!(crate::encrypt::restore_backup(&path, &junk, &key()).is_err());
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "a refused restore changed the store"
    );
    let names = names(path.parent().unwrap());
    assert!(
        names
            .iter()
            .all(|n| !n.ends_with(".restoring") && !n.ends_with(".before-restore")),
        "{names:?}"
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}
