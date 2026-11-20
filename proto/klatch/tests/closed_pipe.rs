//! A command whose reader closes the pipe early ends quietly (#846): `klatch who lobby | head` must not print a panic.
mod common;

use common::*;
use std::process::{Command, ExitStatus, Stdio};

/// Run `klatch args` with its standard output already closed at the other end, as `head` leaves it.
fn run_with_closed_stdout(h: &Home, who: &str, args: &[&str]) -> (ExitStatus, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env_remove("KLATCH_IDENTITY_FILE")
        .env("KLATCH_HOME", &h.0)
        .env("KLATCH_KEY_DIR", h.0.join("keys"))
        .env("KLATCH_NAME", who)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    (
        out.status,
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn a_closed_stdout_ends_quietly_without_a_panic_message() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    for i in 0..5 {
        h.ok("bob", &["post", "r", &format!("message number {i}")]);
    }
    let (status, err) = run_with_closed_stdout(&h, "alice", &["who", "r"]);
    assert!(!err.contains("internal error"), "stderr: {err}");
    assert!(!err.contains("Broken pipe"), "stderr: {err}");
    assert_eq!(status.code(), Some(141), "ends as a closed pipe does");
}
