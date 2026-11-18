//! A failed harness probe turns the feature it guards off, and doctor says why (#482). The one feature that depends
//! on the Claude Code version is the wake watcher (`asyncRewake`). The probe is `claude --version`; every test puts a
//! fake `claude` first on a PATH of its own. A probe that cannot tell (no claude, no version in the answer) turns
//! nothing off.
mod common;

use common::*;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Instant;

/// A directory holding a `claude` that prints `answer` for `--version`.
fn fake_claude(dir: &Path, answer: &str) -> PathBuf {
    let bin_dir = dir.join("fakebin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let bin = bin_dir.join("claude");
    std::fs::write(&bin, format!("#!/bin/sh\nprintf '%s\\n' '{answer}'\n")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin_dir
}

fn path_with(first: Option<&Path>) -> String {
    let base = "/usr/bin:/bin";
    first.map_or(base.to_string(), |d| format!("{}:{base}", d.display()))
}

fn doctor(s: &Setup, claude_dir: Option<&Path>) -> String {
    let o = Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env_remove("KLATCH_IDENTITY_FILE")
        .env_clear()
        .current_dir(s.project())
        .env("HOME", s.home())
        .env("KLATCH_KEY_DIR", s.home().join("keys"))
        .env("PATH", path_with(claude_dir))
        .arg("doctor")
        .output()
        .unwrap();
    out(&o) + &String::from_utf8_lossy(&o.stderr)
}

fn line<'a>(t: &'a str, prefix: &str) -> &'a str {
    t.lines()
        .find(|l| l.contains(prefix))
        .unwrap_or_else(|| panic!("no '{prefix}' line in:\n{t}"))
}

#[test]
fn doctor_says_the_wake_hook_is_off_when_claude_is_older_than_the_floor() {
    let s = Setup::new();
    let dir = fake_claude(&s.home(), "2.0.7 (Claude Code)");
    let t = doctor(&s, Some(&dir));
    let l = line(&t, "harness:");
    assert!(l.starts_with("warn"), "{t}");
    assert!(l.contains("2.0.7") && l.contains("2.1.319"), "{t}");
    assert!(l.contains("wake hook is off"), "{t}");
    assert!(t.contains("fix:") && t.contains("upgrade"), "{t}");
}

#[test]
fn doctor_says_the_wake_hook_is_on_for_a_claude_at_or_above_the_floor() {
    let s = Setup::new();
    let dir = fake_claude(&s.home(), "2.1.325 (Claude Code)");
    let t = doctor(&s, Some(&dir));
    let l = line(&t, "harness:");
    assert!(l.starts_with("ok"), "{t}");
    assert!(l.contains("2.1.325") && l.contains("asyncRewake"), "{t}");
}

#[test]
fn doctor_with_no_claude_or_an_unreadable_answer_turns_nothing_off() {
    let s = Setup::new();
    let t = doctor(&s, None);
    let l = line(&t, "harness:");
    assert!(l.starts_with("ok") && l.contains("not on the PATH"), "{t}");

    let dir = fake_claude(&s.home(), "this is not a version");
    let t = doctor(&s, Some(&dir));
    let l = line(&t, "harness:");
    assert!(l.starts_with("warn"), "{t}");
    assert!(
        l.contains("left as it is"),
        "unknown must not turn it off: {t}"
    );
}

/// Run `klatch wake --timeout 6` as `bob` with `claude_dir` first on the PATH.
fn wake(h: &Home, claude_dir: Option<&Path>) -> (Output, f64) {
    let started = Instant::now();
    let o = Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env_remove("KLATCH_IDENTITY_FILE")
        .env("KLATCH_HOME", &h.0)
        .env("KLATCH_KEY_DIR", h.0.join("keys"))
        .env("KLATCH_NAME", "bob")
        .env("PATH", path_with(claude_dir))
        .args(["wake", "--timeout", "6"])
        .output()
        .unwrap();
    (o, started.elapsed().as_secs_f64())
}

fn with_unread() -> Home {
    let h = Home::new();
    h.ok("bob", &["join", "r"]);
    h.ok("alice", &["join", "r"]);
    h.ok("alice", &["post", "r", "hello bob"]);
    h
}

#[test]
fn the_wake_watcher_exits_at_once_and_quietly_when_claude_is_too_old() {
    let h = with_unread();
    let dir = fake_claude(&h.0, "2.0.7 (Claude Code)");
    let (o, secs) = wake(&h, Some(&dir));
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(secs < 5.0, "it watched for {secs}s");
    let e = String::from_utf8_lossy(&o.stderr);
    assert!(e.contains("wake hook is off") && e.contains("2.0.7"), "{e}");
    assert!(!e.contains("unread"), "no wake pointer: {e}");
}

#[test]
fn the_wake_watcher_still_wakes_on_a_supported_claude_and_when_the_probe_cannot_tell() {
    let h = with_unread();
    let dir = fake_claude(&h.0, "2.1.325 (Claude Code)");
    let (o, _) = wake(&h, Some(&dir));
    assert_eq!(
        o.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );

    let h = with_unread();
    let (o, _) = wake(&h, None);
    assert_eq!(
        o.status.code(),
        Some(2),
        "no claude on the path must not turn it off"
    );
}
