//! Tests for the branches of key custody that a mutation run found unpinned (#724, from #407): restoring a recovery
//! code, finishing a crashed rotation, where key files live, and the label key commands. They use temporary homes
//! only. A function that reads the key directory from the environment (`restore_key`, the label commands,
//! `state_dir`) runs in a child process of the test binary with its own `HOME`, `XDG_STATE_HOME` and `KLATCH_KEY_DIR`,
//! so no test reaches the Keychain or the real store, whatever the caller's environment is.

use super::*;
use klatch::keys::FileKeys;
use std::process::Command;

const NAME: &str = "store-test";

fn scratch(tag: &str) -> crate::test_dir::TestDir {
    crate::test_dir::TestDir::new(&format!("survivor-{tag}"))
}

fn file_home(d: &Path) -> KeyHome {
    KeyHome {
        source: Box::new(FileKeys::new(&d.join("keys"))),
        name: NAME.into(),
        warning: None,
    }
}

fn same(a: &Key, b: &Key) -> bool {
    a == b
}

/// Run the test `name` of this module in a child process whose environment is exactly the one given.
fn in_child(name: &str, dir: &Path, extra: &[(&str, &str)]) {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args([
        "--exact",
        &format!("custody::survivor_tests::{name}"),
        "--test-threads=1",
    ])
    .env_clear()
    .env("KLATCH_SURVIVOR_DIR", dir)
    .env("KLATCH_KEY_DIR", dir.join("keys"))
    .env("HOME", dir.join("home"))
    .env("TMPDIR", std::env::temp_dir());
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{name} failed in the child:\n{text}");
    assert!(
        text.contains("test result: ok. 1 passed"),
        "{name} did not run in the child:\n{text}"
    );
}

/// The directory a child test was given; `None` in the parent, where the child tests do nothing.
fn child_dir() -> Option<PathBuf> {
    std::env::var_os("KLATCH_SURVIVOR_DIR").map(PathBuf::from)
}

fn rekeyed_but_not_promoted(d: &Path) -> (PathBuf, Key, Key) {
    let db = d.join("klatch.db");
    let kh = file_home(d);
    let store = open_store_in(&db, &kh).unwrap();
    let old = kh.source.get(NAME).unwrap().unwrap();
    let new = Key::generate().unwrap();
    kh.source.put(&next_name(NAME), &new).unwrap();
    store.rekey(&new).unwrap();
    drop(store);
    (db, old, new)
}

#[test]
fn a_file_with_no_bytes_is_not_data_and_one_byte_is() {
    let d = scratch("has-data");
    let db = d.join("klatch.db");
    assert!(!has_data(&db), "a missing file has data");
    std::fs::write(&db, b"").unwrap();
    assert!(!has_data(&db), "an empty file has data");
    std::fs::write(&db, b"x").unwrap();
    assert!(has_data(&db));
}

#[test]
fn an_empty_store_file_gets_a_key_and_opens_instead_of_being_called_lost() {
    let d = scratch("empty-db");
    let db = d.join("klatch.db");
    std::fs::write(&db, b"").unwrap();
    let kh = file_home(&d);
    open_store_in(&db, &kh).expect("an empty file is a store that was never written");
    assert!(kh.source.get(NAME).unwrap().is_some());
}

#[test]
fn a_key_opens_a_store_only_when_it_is_missing_empty_plaintext_or_really_opened_by_it() {
    let d = scratch("opens-existing");
    let db = d.join("klatch.db");
    let kh = file_home(&d);
    let right = Key::generate().unwrap();
    assert!(
        opens_existing_store(&db, &right),
        "no store yet is nothing to contradict"
    );
    let _ = std::fs::remove_file(&db);
    std::fs::write(
        &db,
        b"SQLite format 3\0 and the rest of a plaintext database",
    )
    .unwrap();
    assert!(
        opens_existing_store(&db, &right),
        "a plaintext database is converted, not refused"
    );
    std::fs::remove_file(&db).unwrap();
    drop(SqliteStore::open(&db, &right).unwrap());
    assert!(opens_existing_store(&db, &right));
    let wrong = Key::generate().unwrap();
    assert!(
        !opens_existing_store(&db, &wrong),
        "a key that does not open the encrypted store was accepted"
    );
    let _ = kh;
}

#[test]
fn the_directory_lock_is_taken_only_where_the_store_directory_exists() {
    let d = scratch("lock-dir");
    assert!(lock_store_dir(&d.join("klatch.db")).unwrap().is_some());
    let missing = d.join("not-yet").join("klatch.db");
    assert!(lock_store_dir(&missing).unwrap().is_none());
    assert!(!d.join("not-yet").exists(), "the lock made the directory");
}

#[test]
fn opening_beside_a_rotation_in_a_directory_that_does_not_exist_skips_the_directory_lock() {
    let d = scratch("beside-missing");
    let db = d.join("fresh").join("klatch.db");
    let key = Key::generate().unwrap();
    // there is nothing to lock, so the failure is the database's own, not a failed attempt to lock a directory
    let err = open_beside_rotation(&db, &file_home(&d), key)
        .err()
        .expect("sqlite does not make a directory")
        .to_string();
    assert!(!err.contains("store directory"), "{err}");
    assert!(err.contains("unable to open"), "{err}");
}

#[test]
fn a_rotation_that_died_after_the_store_was_rekeyed_is_finished_by_the_next_open() {
    let d = scratch("crashed");
    let (db, old, new) = rekeyed_but_not_promoted(&d);
    let kh = file_home(&d);
    assert!(same(&kh.source.get(NAME).unwrap().unwrap(), &old));
    open_store_in(&db, &kh).expect("the repair opens the store with the second key");
    assert!(
        same(&kh.source.get(NAME).unwrap().unwrap(), &new),
        "the second key was not promoted"
    );
    assert!(
        kh.source.get(&next_name(NAME)).unwrap().is_none(),
        "the second key was left behind"
    );
}

#[test]
fn a_second_key_equal_to_the_current_one_never_repairs_anything_and_is_left_alone() {
    let d = scratch("next-equal");
    let db = d.join("klatch.db");
    let kh = file_home(&d);
    drop(open_store_in(&db, &kh).unwrap());
    let current = kh.source.get(NAME).unwrap().unwrap();
    kh.source.put(&next_name(NAME), &current).unwrap();
    // a store that opens with neither key: the repair must not run on a leftover equal to the key in use
    let other = Key::generate().unwrap();
    let broken = d.join("other.db");
    drop(SqliteStore::open(&broken, &other).unwrap());
    assert!(open_keyed(&broken, &kh, current).is_err());
    assert!(
        kh.source.get(&next_name(NAME)).unwrap().is_some(),
        "the leftover was deleted by a repair that did not happen"
    );
}

#[test]
fn rotating_again_after_a_crash_before_the_store_was_rekeyed_reuses_the_stored_second_key() {
    let d = scratch("rot-resume");
    let db = d.join("klatch.db");
    let kh = file_home(&d);
    drop(open_store_in(&db, &kh).unwrap());
    let stored = Key::generate().unwrap();
    kh.source.put(&next_name(NAME), &stored).unwrap();
    let new = rotate_key_in(&db, &kh).unwrap();
    assert!(same(&new, &stored), "a fresh key replaced the stored one");
    assert!(same(&kh.source.get(NAME).unwrap().unwrap(), &stored));
    drop(open_store_in(&db, &kh).unwrap());
}

#[test]
fn a_second_key_equal_to_the_current_one_is_not_rotated_to_again() {
    let d = scratch("rot-equal");
    let db = d.join("klatch.db");
    let kh = file_home(&d);
    drop(open_store_in(&db, &kh).unwrap());
    let old = kh.source.get(NAME).unwrap().unwrap();
    kh.source.put(&next_name(NAME), &old).unwrap();
    let new = rotate_key_in(&db, &kh).unwrap();
    assert!(!same(&new, &old), "the rotation kept the same key");
    assert!(same(&kh.source.get(NAME).unwrap().unwrap(), &new));
    assert!(kh.source.get(&next_name(NAME)).unwrap().is_none());
    drop(open_store_in(&db, &kh).unwrap());
}

#[test]
fn state_dir_follows_xdg_state_home_then_home_and_never_the_working_directory() {
    let d = scratch("state-dir");
    let home = d.join("home");
    let xdg = d.join("xdg");
    in_child(
        "child_state_dir",
        &d,
        &[(
            "KLATCH_EXPECT_STATE",
            home.join(".local/state/klatch/keys").to_str().unwrap(),
        )],
    );
    in_child(
        "child_state_dir",
        &d,
        &[
            ("XDG_STATE_HOME", xdg.to_str().unwrap()),
            (
                "KLATCH_EXPECT_STATE",
                xdg.join("klatch/keys").to_str().unwrap(),
            ),
        ],
    );
}

#[test]
fn child_state_dir() {
    let Some(_) = child_dir() else { return };
    let expect = PathBuf::from(std::env::var_os("KLATCH_EXPECT_STATE").unwrap());
    assert_eq!(state_dir(), expect);
    assert!(state_dir().is_absolute());
}

#[test]
fn restoring_a_code_that_does_not_open_the_store_changes_nothing_and_refuses() {
    let d = scratch("restore");
    in_child("child_restore_key", &d, &[]);
}

#[test]
fn child_restore_key() {
    let Some(d) = child_dir() else { return };
    let db = d.join("klatch.db");
    drop(open_store(&db).unwrap());
    let good = current_key(&db).unwrap().unwrap();
    let wrong = Key::generate().unwrap();
    let err = restore_key(&db, &wrong).unwrap_err().to_string();
    assert!(err.contains("does not open this store"), "{err}");
    assert!(
        same(&current_key(&db).unwrap().unwrap(), &good),
        "a refused restore changed the key"
    );
    // the right key, already in place, is fine
    restore_key(&db, &good).unwrap();
    // a key that was lost is put back by its code
    let kh = key_home(&db);
    kh.source.delete(&kh.name).unwrap();
    assert!(current_key(&db).unwrap().is_none());
    restore_key(&db, &good).unwrap();
    assert!(same(&current_key(&db).unwrap().unwrap(), &good));
    // a different key already in the keystore is never replaced
    let other = Key::generate().unwrap();
    kh.source.put(&kh.name, &other).unwrap();
    let err = restore_key(&db, &good).unwrap_err().to_string();
    assert!(err.contains("already has a different key"), "{err}");
    assert!(same(&current_key(&db).unwrap().unwrap(), &other));
    // with no store yet there is nothing to contradict
    let fresh = d.join("fresh").join("klatch.db");
    std::fs::create_dir_all(fresh.parent().unwrap()).unwrap();
    let kh2 = key_home(&fresh);
    restore_key(&fresh, &wrong).unwrap();
    assert!(same(&kh2.source.get(&kh2.name).unwrap().unwrap(), &wrong));
}

#[test]
fn the_label_key_commands_make_show_load_and_restore_the_key_of_a_sealed_label() {
    let d = scratch("labels");
    in_child("child_label_keys", &d, &[]);
}

#[test]
fn child_label_keys() {
    let Some(d) = child_dir() else { return };
    let db = d.join("klatch.db");
    assert!(label_key(&db, "private").unwrap().is_none());
    assert!(ensure_label_key(&db, "private").unwrap(), "no key was made");
    assert!(
        !ensure_label_key(&db, "private").unwrap(),
        "a second call made another key"
    );
    let made = label_key(&db, "private")
        .unwrap()
        .expect("the key is shown");
    let ring = label_ring(&db, &["private".to_string(), "secret".to_string()]).unwrap();
    assert_eq!(ring.labels(), vec!["private"]);
    assert!(same(ring.get("private").unwrap(), &made));
    // restoring the same key is fine, a different key over an existing one is refused, a missing one is put back
    restore_label_key(&db, "private", &made).unwrap();
    let other = Key::generate().unwrap();
    let err = restore_label_key(&db, "private", &other)
        .unwrap_err()
        .to_string();
    assert!(err.contains("already has a different key"), "{err}");
    assert!(same(&label_key(&db, "private").unwrap().unwrap(), &made));
    restore_label_key(&db, "secret", &other).unwrap();
    assert!(same(&label_key(&db, "secret").unwrap().unwrap(), &other));
    let ring = label_ring(&db, &["private".to_string(), "secret".to_string()]).unwrap();
    assert_eq!(ring.labels(), vec!["private", "secret"]);
}
