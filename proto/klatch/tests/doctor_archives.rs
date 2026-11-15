//! `klatch doctor` lists the archive files under the klatch directory and checks that each opens with the store key and
//! has not been tampered with (#487). It reads counts only, and creates nothing.
mod common;

use common::*;
use klatch::keys::KeySource;
use klatch::store::{SqliteStore, Store};
use std::path::PathBuf;
use std::process::Command;

fn klatch_dir(s: &Setup) -> PathBuf {
    s.home().join(".klatch")
}

fn run(s: &Setup, name: Option<&str>, args: &[&str]) -> std::process::Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_klatch"));
    c.current_dir(s.project())
        .env_clear()
        .env("HOME", s.home())
        .env("KLATCH_KEY_DIR", s.home().join("keys"))
        .args(args);
    if let Some(n) = name {
        c.env("KLATCH_NAME", n);
    }
    c.output().unwrap()
}

fn doctor(s: &Setup) -> String {
    let o = run(s, None, &["doctor"]);
    out(&o) + &String::from_utf8_lossy(&o.stderr)
}

/// A real store made by the binary (so its key is in the key directory), with three old messages archived into
/// `archive/<name>` the way `klatch admin archive` does it.
fn store_with_archive(s: &Setup, name: &str) -> PathBuf {
    assert!(run(s, Some("alice"), &["join", "lobby"]).status.success());
    let dir = klatch_dir(s);
    let key = klatch::keys::FileKeys::new(&s.home().join("keys"))
        .get(&klatch::keys::store_key_name(&dir))
        .unwrap()
        .expect("the binary made a key");
    let store = SqliteStore::open(&dir.join("rooms.db"), &key).unwrap();
    for (i, body) in ["one", "two", "three"].iter().enumerate() {
        let seat = klatch::store::Seat::new("lobby", "alice");
        store.post(seat, body, 10 + i as i64).unwrap();
    }
    std::fs::create_dir_all(dir.join("archive")).unwrap();
    let file = dir.join("archive").join(name);
    assert_eq!(store.archive_before("lobby", 500, &file, &key).unwrap(), 3);
    file
}

fn archives_line(t: &str) -> &str {
    t.lines()
        .find(|l| l.contains("archives:"))
        .unwrap_or_else(|| panic!("no 'archives' line in:\n{t}"))
}

#[test]
fn a_machine_with_no_archives_says_nothing_about_them() {
    let s = Setup::new();
    let t = doctor(&s);
    assert!(!t.contains("archives:"), "{t}");
}

#[test]
fn a_good_archive_is_listed_as_ok_with_its_counts() {
    let s = Setup::new();
    store_with_archive(&s, "lobby-100.db");
    let t = doctor(&s);
    let l = archives_line(&t);
    assert!(l.starts_with("ok"), "{t}");
    assert!(l.contains("1 file") && l.contains("3 messages"), "{t}");
    assert!(!t.contains("0 warnings") || t.contains("0 failures"), "{t}");
}

#[test]
fn a_tampered_archive_is_a_failure_that_names_the_file_and_the_fix() {
    let s = Setup::new();
    let file = store_with_archive(&s, "lobby-100.db");
    let mut bytes = std::fs::read(&file).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0xff;
    std::fs::write(&file, bytes).unwrap();
    let t = doctor(&s);
    assert!(
        t.lines()
            .any(|l| l.starts_with("FAIL") && l.contains("lobby-100.db")),
        "{t}"
    );
    assert!(t.contains("fix:"), "{t}");
    assert!(t.contains("1 failures"), "{t}");
}

#[test]
fn an_interrupted_archive_leaves_a_warning_not_a_failure() {
    let s = Setup::new();
    let file = store_with_archive(&s, "lobby-100.db");
    std::fs::write(file.with_file_name("lobby-200.db.partial"), "half").unwrap();
    let t = doctor(&s);
    assert!(
        t.lines()
            .any(|l| l.starts_with("warn") && l.contains("lobby-200.db.partial")),
        "{t}"
    );
    assert!(t.contains("0 failures"), "{t}");
}

#[test]
fn doctor_adds_no_files_to_the_archive_directory() {
    let s = Setup::new();
    let file = store_with_archive(&s, "lobby-100.db");
    let before = std::fs::read_dir(file.parent().unwrap()).unwrap().count();
    doctor(&s);
    doctor(&s);
    assert_eq!(
        std::fs::read_dir(file.parent().unwrap()).unwrap().count(),
        before
    );
}
