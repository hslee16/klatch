//! `klatch doctor` warns while the failed proofs of the last 24 hours, across all terminals, are at or above the
//! alarm level (#1310), and the audit gets one `proof-failed-many` row when the level is reached. It is an alarm
//! only: nothing is locked. The real binary, on a pseudo-terminal for every wrong passphrase (each is a terminal
//! of its own), with throwaway passphrases.
mod common;

use common::admin::*;
use common::pty::*;
use common::*;
use std::process::Command;

/// The level of the alarm, as the product states it in its documentation (`proof::alarm::CEILING`).
const LEVEL: usize = klatch::proof::alarm::CEILING as usize;

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

fn alarm_lines(t: &str) -> Vec<&str> {
    t.lines().filter(|l| l.contains("proof failures")).collect()
}

fn fail_once(h: &Home) {
    let r = run_on_pty(admin_cmd(h, &["pause"]), &[WRONG]);
    assert!(!r.ok(), "{}", r.text);
}

fn audit(h: &Home) -> String {
    let r = run_on_pty(admin_cmd(h, &["audit", "--limit", "500"]), &[PASS]);
    assert!(r.ok(), "{}", r.text);
    r.text
}

#[test]
fn doctor_is_quiet_below_the_level_warns_at_it_and_says_counts_only() {
    let h = Home::new();
    h.set_passphrase();
    for _ in 0..LEVEL - 1 {
        fail_once(&h);
    }
    let t = doctor(&h);
    assert!(alarm_lines(&t).is_empty(), "{t}");
    assert!(!audit(&h).contains("proof-failed-many"));
    fail_once(&h);
    let t = doctor(&h);
    let l = alarm_lines(&t);
    assert_eq!(l.len(), 1, "{t}");
    assert!(l[0].starts_with("warn"), "{t}");
    assert!(l[0].contains(&format!("{LEVEL} wrong")), "{t}");
    assert!(
        l[0].contains("terminals") && l[0].contains("nothing is locked"),
        "{t}"
    );
    for secret in [PASS, WRONG, NEW] {
        assert!(!t.contains(secret), "{secret} leaked into {t}");
    }
    let rows = audit(&h);
    assert_eq!(rows.matches("proof-failed-many").count(), 1, "{rows}");
    fail_once(&h);
    assert_eq!(
        audit(&h).matches("proof-failed-many").count(),
        1,
        "one row for the crossing, not one for every failure"
    );
    let t = doctor(&h);
    assert!(
        alarm_lines(&t)[0].contains(&format!("{} wrong", LEVEL + 1)),
        "{t}"
    );
}

#[test]
fn doctor_says_nothing_about_failures_when_the_store_key_cannot_be_read() {
    let h = Home::new();
    h.set_passphrase();
    for _ in 0..LEVEL {
        fail_once(&h);
    }
    assert_eq!(alarm_lines(&doctor(&h)).len(), 1);
    // without the key the store cannot be read: the store line says so and the failure line is not guessed at
    std::fs::remove_dir_all(h.0.join("keys")).unwrap();
    let t = doctor(&h);
    assert!(alarm_lines(&t).is_empty(), "{t}");
    assert!(t.contains("store:"), "{t}");
}
