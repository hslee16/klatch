//! The checks inside the conversion path that the store-level tests cannot reach (#721): the copy verification, the
//! scrub loop, the row count of a conversion and the directory lock for a bare file name. Each test pins one thing a
//! mutant of the function once got away with (see proto/MUTANTS-RESULTS.md).

use super::*;
use crate::store::insecure_test_key;
use std::sync::mpsc;

const VERSION: i64 = 7;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("klatch-store-enc-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A plaintext database of two tables (3 and 300 rows, the second spanning many pages) at `VERSION`.
fn plaintext_db(dir: &Path) -> PathBuf {
    let path = dir.join("rooms.db");
    let c = Connection::open(&path).unwrap();
    c.execute_batch("CREATE TABLE small(a INTEGER); CREATE TABLE big(a INTEGER, pad TEXT);")
        .unwrap();
    for i in 0..3 {
        c.execute("INSERT INTO small VALUES (?1)", params![i])
            .unwrap();
    }
    let pad = "x".repeat(100);
    for i in 0..300 {
        c.execute("INSERT INTO big VALUES (?1, ?2)", params![i, pad])
            .unwrap();
    }
    c.pragma_update(None, "user_version", VERSION).unwrap();
    path
}

/// The converted, encrypted store: 2 tables, `VERSION`, many pages.
fn encrypted_db(dir: &Path) -> PathBuf {
    let path = plaintext_db(dir);
    ensure_encrypted(&path, &insecure_test_key()).unwrap();
    path
}

fn refusal(r: Result<()>) -> String {
    r.expect_err("verify_copy accepted a bad copy").to_string()
}

#[test]
fn a_conversion_reports_its_table_and_row_counts() {
    let dir = scratch("counts");
    let path = plaintext_db(&dir);
    let outcome = ensure_encrypted(&path, &insecure_test_key()).unwrap();
    assert_eq!(
        outcome,
        Outcome::Encrypted {
            tables: 2,
            rows: 303
        }
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_good_copy_passes_verification() {
    let dir = scratch("verify-good");
    let path = encrypted_db(&dir);
    verify_copy(&path, &insecure_test_key(), VERSION, 2).unwrap();
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_copy_with_a_missing_table_is_refused_on_that_alone() {
    let dir = scratch("verify-tables");
    let path = encrypted_db(&dir);
    // the copy holds 2 tables; the source had 3: only the table count is wrong
    let msg = refusal(verify_copy(&path, &insecure_test_key(), VERSION, 3));
    assert!(msg.contains("2 tables of 3"), "{msg}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_copy_with_the_wrong_schema_version_is_refused_on_that_alone() {
    let dir = scratch("verify-version");
    let path = encrypted_db(&dir);
    let msg = refusal(verify_copy(&path, &insecure_test_key(), VERSION + 1, 2));
    assert!(msg.contains("version 7 of 8"), "{msg}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_copy_with_a_damaged_page_is_refused_on_that_alone() {
    let dir = scratch("verify-page");
    let path = encrypted_db(&dir);
    let mut bytes = std::fs::read(&path).unwrap();
    assert!(bytes.len() > 5 * 4096, "the copy is too small to have a later page");
    bytes[3 * 4096 + 100] ^= 0xFF; // a data page, not the schema page the table count reads
    std::fs::write(&path, bytes).unwrap();
    let msg = refusal(verify_copy(&path, &insecure_test_key(), VERSION, 2));
    assert!(msg.contains("2 tables of 2, version 7 of 7"), "{msg}");
    assert!(!msg.contains(", 0 page problems"), "{msg}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_copy_under_another_key_is_refused() {
    let dir = scratch("verify-key");
    let path = encrypted_db(&dir);
    let other = Key::from_bytes([9; 32]);
    assert!(verify_copy(&path, &other, VERSION, 2).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

/// Run the scrub on its own thread so a loop that never ends fails the test instead of hanging it.
fn scrub_within(path: &Path, secs: u64) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    let p = path.to_owned();
    std::thread::spawn(move || {
        let _ = tx.send(scrub_and_remove(&p));
    });
    rx.recv_timeout(Duration::from_secs(secs))
        .expect("the scrub did not finish")
}

#[test]
fn a_file_of_several_pages_is_zeroed_to_its_exact_length_before_removal() {
    let dir = scratch("scrub-pages");
    let (a, b) = (dir.join("a"), dir.join("b"));
    // two full 8 KiB chunks and a partial third: the loop must end exactly at the length
    let len = 8192 * 2 + 5;
    std::fs::write(&a, vec![0xAB; len]).unwrap();
    std::fs::hard_link(&a, &b).unwrap(); // the same inode under a second name, to read it after the removal
    scrub_within(&a, 30).unwrap();
    assert!(!a.exists());
    let left = std::fs::read(&b).unwrap();
    assert_eq!(left.len(), len, "the scrub changed the length");
    assert!(left.iter().all(|x| *x == 0), "some bytes were not zeroed");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_file_of_exactly_one_chunk_is_zeroed() {
    let dir = scratch("scrub-chunk");
    let (a, b) = (dir.join("a"), dir.join("b"));
    std::fs::write(&a, vec![0xCD; 8192]).unwrap();
    std::fs::hard_link(&a, &b).unwrap();
    scrub_within(&a, 30).unwrap();
    let left = std::fs::read(&b).unwrap();
    assert_eq!(left.len(), 8192);
    assert!(left.iter().all(|x| *x == 0));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_empty_file_is_removed_and_a_missing_one_is_an_error() {
    let dir = scratch("scrub-empty");
    let a = dir.join("a");
    std::fs::write(&a, b"").unwrap();
    scrub_within(&a, 30).unwrap();
    assert!(!a.exists());
    assert!(scrub_within(&dir.join("nope"), 30).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_store_given_as_a_bare_file_name_locks_the_current_directory() {
    // the parent of "rooms.db" is the empty path, which means the current directory, not an error
    let f = lock_dir_shared(Path::new("rooms.db")).expect("a bare file name has no directory to open");
    drop(f);
}

#[test]
fn the_lock_is_on_the_directory_that_holds_the_store_not_on_the_current_one() {
    let dir = scratch("lock-dir");
    assert!(lock_dir_shared(&dir.join("rooms.db")).is_ok());
    // a directory that does not exist is an error; the current directory is not a fallback for it
    assert!(lock_dir_shared(&dir.join("missing").join("rooms.db")).is_err());
    std::fs::remove_dir_all(&dir).ok();
}
