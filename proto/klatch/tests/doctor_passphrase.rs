//! `klatch doctor` warns while no admin passphrase is set (#1262). It only reads: it never sets, issues or asks for
//! anything, and a warning leaves the exit code at 0 (only a failed probe exits 1). The real binary, with throwaway
//! passphrases.
mod common;

use common::*;
use std::process::{Command, Output};

fn doctor(h: &Home) -> Output {
    Command::new(env!("CARGO_BIN_EXE_klatch"))
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
