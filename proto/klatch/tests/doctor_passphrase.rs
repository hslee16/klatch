//! `klatch doctor` warns while no admin passphrase is set (#1262). It only reads: it never sets, issues or asks for
//! anything, and a warning leaves the exit code at 0 (only a failed probe exits 1). The real binary, with throwaway
//! passphrases.
mod common;

use common::*;
use std::process::{Command, Output};

fn doctor(h: &Home) -> Output {
    Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env_remove("KLATCH_IDENTITY_FILE")
        .env_clear()
        .current_dir(&h.0)
        .env("HOME", &h.0)
        .env("KLATCH_HOME", &h.0)
        .env("KLATCH_KEY_DIR", h.0.join("keys"))
        .arg("doctor")
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    out(o) + &String::from_utf8_lossy(&o.stderr)
}

fn lines(t: &str) -> Vec<&str> {
    t.lines()
        .filter(|l| l.contains("admin passphrase"))
        .collect()
}

#[test]
fn doctor_warns_when_a_store_exists_and_no_passphrase_is_set_and_names_the_fix() {
    let h = Home::new();
    assert!(h.run("a", &["join", "lobby"]).status.success());
    let o = doctor(&h);
    let t = text(&o);
    let l = lines(&t);
    assert_eq!(l.len(), 1, "{t}");
    assert!(l[0].starts_with("warn"), "{t}");
    assert!(l[0].contains("no admin passphrase is set"), "{t}");
    assert!(
        t.contains("fix: run `klatch admin init-passphrase` yourself, in a terminal"),
        "{t}"
    );
    assert!(
        o.status.success(),
        "a warning must not change the exit code: {t}"
    );
    assert!(!t.contains('\u{2014}'), "{t}");
}

#[test]
fn doctor_is_quiet_about_the_passphrase_once_one_is_set() {
    let h = Home::new();
    h.set_passphrase();
    let o = doctor(&h);
    let t = text(&o);
    assert!(lines(&t).is_empty(), "{t}");
    assert!(o.status.success(), "{t}");
    assert!(!t.contains(PASS), "{t}");
}

#[test]
fn doctor_says_nothing_about_the_passphrase_with_no_store_and_creates_nothing() {
    let h = Home::new();
    let t = text(&doctor(&h));
    assert!(lines(&t).is_empty(), "{t}");
    assert!(!h.0.join("rooms.db").exists(), "{t}");
}

#[test]
fn doctor_does_not_set_the_passphrase_by_warning_about_it() {
    let h = Home::new();
    assert!(h.run("a", &["join", "lobby"]).status.success());
    let _ = doctor(&h);
    let again = text(&doctor(&h));
    assert_eq!(lines(&again).len(), 1, "still unset after doctor: {again}");
}

/// Replace the store key file of `h` with 32 other bytes, so the key exists but does not open the store (#1585).
fn swap_key_for_a_wrong_one(h: &Home) {
    let dir = h.0.join("keys");
    let files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| std::fs::metadata(p).unwrap().len() == 32)
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    std::fs::write(&files[0], [7u8; 32]).unwrap();
}

#[test]
fn doctor_fails_when_the_key_file_does_not_open_the_store_and_says_so() {
    let h = Home::new();
    assert!(h.run("a", &["join", "lobby"]).status.success());
    swap_key_for_a_wrong_one(&h);
    let o = doctor(&h);
    let t = text(&o);
    let store: Vec<_> = t.lines().filter(|l| l.contains("store:")).collect();
    assert_eq!(store.len(), 1, "{t}");
    assert!(store[0].starts_with("FAIL"), "{t}");
    assert!(store[0].contains("does not open the store"), "{t}");
    assert!(!o.status.success(), "{t}");
    assert!(!t.contains("the key is in key files"), "{t}");
    // the other store lines are not silently skipped: they say why
    assert!(
        lines(&t).iter().all(|l| !l.contains("no admin passphrase")),
        "an unreadable store must not be reported as having no passphrase: {t}"
    );
}

#[test]
fn doctor_gives_the_wrong_key_hint_only_for_a_wrong_key() {
    let h = Home::new();
    assert!(h.run("a", &["join", "lobby"]).status.success());
    swap_key_for_a_wrong_one(&h);
    let t = text(&doctor(&h));
    assert!(t.contains("this is the wrong key for this store"), "{t}");
}

#[cfg(unix)]
#[test]
fn doctor_does_not_call_an_unreadable_store_a_wrong_key() {
    use std::os::unix::fs::PermissionsExt;
    let h = Home::new();
    assert!(h.run("a", &["join", "lobby"]).status.success());
    let db = h.0.join("rooms.db");
    std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o000)).unwrap();
    // root reads a mode-000 file, so there is nothing to see as root (#1709)
    if std::fs::File::open(&db).is_ok() {
        std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o600)).unwrap();
        eprintln!("skipped: this user can read a mode-000 file");
        return;
    }
    let o = doctor(&h);
    let t = text(&o);
    std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(o.status.code(), Some(1), "{t}");
    assert!(
        t.lines()
            .any(|l| l.contains("store:") && l.starts_with("FAIL")),
        "{t}"
    );
    assert!(!t.contains("wrong key"), "{t}");
    assert!(!t.contains("klatch admin recover"), "{t}");
}
