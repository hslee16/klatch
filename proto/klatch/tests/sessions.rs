//! `klatch sessions` (#131): the live coding-agent sessions on this machine, from the harness registry
//! (`~/.claude/sessions/<pid>.json`), judged live the way the rest of klatch judges it. Read only; never a key file.
mod common;

use common::*;
use std::path::Path;
use std::process::{Command, Stdio};

fn sessions(setup: &Setup, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env_remove("KLATCH_IDENTITY_FILE")
        .env_clear()
        .env("HOME", setup.home())
        .arg("sessions")
        .args(args)
        .output()
        .unwrap()
}

fn registry(setup: &Setup) -> std::path::PathBuf {
    let dir = setup.home().join(".claude").join("sessions");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn domain() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else {
        std::env::consts::OS
    }
}

fn record(pid: u32, start: &str, name: &str) -> String {
    format!(
        r#"{{"pid":{pid},"sessionId":"sess-{pid}","cwd":"/work/project","name":"{name}","procStart":"{start}","pidDomain":"{}"}}"#,
        domain()
    )
}

#[test]
fn no_registry_is_no_sessions_and_not_an_error() {
    let s = Setup::new();
    let o = sessions(&s, &[]);
    assert!(o.status.success());
    assert_eq!(out(&o), "");
    assert!(String::from_utf8_lossy(&o.stderr).contains("no live sessions"));
}

#[test]
fn a_dead_record_and_a_malformed_one_are_ignored_and_counted_and_all_says_why() {
    let s = Setup::new();
    let dir = registry(&s);
    // a pid that no process has, and a record that does not parse
    std::fs::write(
        dir.join("4194000.json"),
        record(4_194_000, "Fri Nov 6 03:04:05 2026", "ghost"),
    )
    .unwrap();
    std::fs::write(dir.join("4194001.json"), "{ not json").unwrap();
    let plain = sessions(&s, &[]);
    assert!(plain.status.success());
    assert_eq!(out(&plain), "");
    let note = String::from_utf8_lossy(&plain.stderr).to_string();
    assert!(
        note.contains("no live sessions") && note.contains("--all"),
        "{note}"
    );
    let all = sessions(&s, &["--all"]);
    let text = out(&all);
    assert!(
        text.contains("ignored\t4194000\tnot running\tghost\t/work/project"),
        "{text}"
    );
    assert!(text.contains("unreadable\t1"), "{text}");
}

#[test]
fn a_key_file_beside_the_records_is_never_read() {
    let s = Setup::new();
    let dir = registry(&s);
    std::fs::write(dir.join("4194002.deadbeef.key"), "TOP-SECRET-TOKEN").unwrap();
    let o = sessions(&s, &["--all"]);
    let everything = out(&o) + &String::from_utf8_lossy(&o.stderr);
    assert!(!everything.contains("TOP-SECRET"), "{everything}");
}

fn wait_until_visible(pid: u32) {
    for _ in 0..100 {
        if !utc_start(pid).is_empty() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("process {pid} never became visible to ps");
}

fn utc_start(pid: u32) -> String {
    let out = Command::new("ps")
        .env("TZ", "UTC")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A real process whose program is called `claude`, so the harness check has something true to find. It is a copy of
/// klatch's own binary (a copied macOS system program such as sleep is killed at launch), serving on a pipe that
/// stays open until the child is killed.
fn fake_claude(dir: &Path) -> std::process::Child {
    let program = dir.join("claude");
    std::fs::copy(env!("CARGO_BIN_EXE_klatch"), &program).unwrap();
    Command::new(&program)
        .args(["serve", "--stdio"])
        .env("KLATCH_NAME", "fake")
        .env("KLATCH_HOME", dir.join("fake-home"))
        .env("KLATCH_KEY_DIR", dir.join("fake-keys"))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

#[test]
fn a_running_session_is_listed_with_its_directory_and_name_and_a_reused_pid_is_not() {
    let s = Setup::new();
    let dir = registry(&s);
    let mut live = fake_claude(&s.home());
    let pid = live.id();
    wait_until_visible(pid);
    std::fs::write(
        dir.join(format!("{pid}.json")),
        record(pid, &utc_start(pid), "klatch-1"),
    )
    .unwrap();
    let o = sessions(&s, &[]);
    live.kill().unwrap();
    live.wait().unwrap();
    let text = out(&o);
    assert!(o.status.success());
    assert_eq!(
        text.trim(),
        format!("{pid}\tklatch-1\t/work/project\tsess-{pid}"),
        "{text}"
    );

    // the same pid with a start time from another day is a reused pid, not this session
    let mut other = fake_claude(&s.home());
    let pid2 = other.id();
    wait_until_visible(pid2);
    std::fs::write(
        dir.join(format!("{pid2}.json")),
        record(pid2, "Mon Jan 1 00:00:00 2001", "stale"),
    )
    .unwrap();
    let o = sessions(&s, &["--all"]);
    other.kill().unwrap();
    other.wait().unwrap();
    let text = out(&o);
    assert!(
        !text.lines().any(|l| l.starts_with(&pid2.to_string())),
        "listed a reused pid: {text}"
    );
    assert!(
        text.contains(&format!(
            "ignored\t{pid2}\tpid reused by another process\tstale"
        )),
        "{text}"
    );
}
