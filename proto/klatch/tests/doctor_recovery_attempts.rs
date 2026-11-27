//! `klatch doctor` shows the recovery attempts recorded beside the store and not yet settled (#1636): failed guesses
//! at the recovery code are otherwise visible only in `<store>.recovery-fails`. The real binary, a throwaway home.
mod common;

use common::*;
use std::process::Command;

fn doctor(h: &Home) -> String {
    let o = Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env_remove("KLATCH_IDENTITY_FILE")
        .current_dir(&h.0)
        .env_clear()
        .env("HOME", &h.0)
        .env("KLATCH_HOME", &h.0)
        .env("KLATCH_KEY_DIR", h.0.join("keys"))
        .arg("doctor")
        .output()
        .unwrap();
    out(&o) + &String::from_utf8_lossy(&o.stderr)
}

fn attempt_lines(t: &str) -> Vec<&str> {
    t.lines()
        .filter(|l| l.contains("recovery attempt"))
        .collect()
}

#[test]
fn doctor_warns_with_the_count_of_recorded_recovery_attempts() {
    let h = Home::new();
    std::fs::write(h.0.join("rooms.db.recovery-fails"), "1\n2\n3\n").unwrap();
    let t = doctor(&h);
    let l = attempt_lines(&t);
    assert_eq!(l.len(), 1, "{t}");
    assert!(l[0].starts_with("warn"), "{t}");
    assert!(l[0].contains("3 recovery attempts are recorded"), "{t}");
    assert!(l[0].contains("rooms.db.recovery-fails"), "{t}");
}

#[test]
fn doctor_says_one_attempt_in_the_singular_and_nothing_without_a_file() {
    let h = Home::new();
    let t = doctor(&h);
    assert!(attempt_lines(&t).is_empty(), "{t}");
    std::fs::write(h.0.join("rooms.db.recovery-fails"), "1\n").unwrap();
    let t = doctor(&h);
    assert!(
        attempt_lines(&t)
            .iter()
            .any(|l| l.contains("1 recovery attempt is recorded")),
        "{t}"
    );
}
