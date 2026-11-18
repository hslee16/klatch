//! A Codex session is a klatch member with no registry record and no typed name (#341, RESEARCH-NOTES 37): the
//! hook's `session_id` and `cwd` on standard input name it, and a command the agent runs names it by
//! `CODEX_THREAD_ID`. `--codex` asks for that; without it nothing changes. A resumed session keeps its member.
mod common;

use common::*;
use std::io::Write;
use std::process::{Command, Output, Stdio};

/// What Codex hands klatch: a hook's JSON on standard input, or the thread id of a command the agent runs.
enum Input<'a> {
    Hook(&'a str),
    Thread(&'a str),
}

/// One klatch call as a Codex session would make it: a clean environment, no typed name, no registry (a HOME with
/// none), the hook's JSON on standard input or the thread id as `CODEX_THREAD_ID`.
fn call(h: &Home, args: &[&str], input: Input) -> Output {
    let (payload, thread) = match input {
        Input::Hook(json) => (json, None),
        Input::Thread(id) => ("", Some(id)),
    };
    let mut c = Command::new(env!("CARGO_BIN_EXE_klatch"));
    c.env_remove("KLATCH_IDENTITY_FILE");
    c.env_clear()
        .env("KLATCH_HOME", &h.0)
        .env("KLATCH_KEY_DIR", h.0.join("keys"))
        .env("HOME", &h.0)
        .current_dir(&h.0)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(t) = thread {
        c.env("CODEX_THREAD_ID", t);
    }
    let mut child = c.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn payload(sid: &str, cwd: &str, source: &str) -> String {
    format!(
        r#"{{"session_id":"{sid}","cwd":"{cwd}","hook_event_name":"SessionStart","source":"{source}","transcript_path":"/x"}}"#
    )
}

fn start(h: &Home, sid: &str, cwd: &str, source: &str) -> Output {
    let p = payload(sid, cwd, source);
    call(
        h,
        &["--codex", "hook", "--event", "SessionStart"],
        Input::Hook(&p),
    )
}

fn members(h: &Home) -> Vec<String> {
    let who = h.ok("observer", &["who", "lobby", "--all"]);
    who.lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|n| n.starts_with("codex"))
        .map(String::from)
        .collect()
}

const THREAD: &str = "019a1b2c-3d4e-7f80-9a0b-1c2d3e4f5a6b";

#[test]
fn a_codex_hook_with_no_name_makes_a_member_from_the_cwd_and_joins_the_lobby() {
    let h = Home::new();
    let o = start(&h, THREAD, "/work/my project", "startup");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(members(&h), vec!["codex-my-project"]);
}

#[test]
fn a_resumed_session_is_the_same_member_and_a_new_session_in_the_same_directory_is_another() {
    let h = Home::new();
    start(&h, THREAD, "/work/proj", "startup");
    // `exec resume` keeps the thread id; the working directory may differ
    start(&h, THREAD, "/elsewhere", "resume");
    assert_eq!(members(&h), vec!["codex-proj"]);
    start(
        &h,
        "019a1b2c-ffff-7f80-9a0b-aaaaaaaaaaaa",
        "/work/proj",
        "startup",
    );
    let m = members(&h);
    assert_eq!(m.len(), 2, "{m:?}");
    assert!(m.contains(&"codex-proj".to_string()), "{m:?}");
}

#[test]
fn a_command_the_agent_runs_is_the_member_its_hook_made_by_the_thread_id() {
    let h = Home::new();
    start(&h, THREAD, "/work/proj", "startup");
    let o = call(
        &h,
        &["--codex", "post", "lobby", "from the agent"],
        Input::Thread(THREAD),
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let read = h.ok("observer", &["read", "lobby"]);
    let line = read.lines().find(|l| l.contains("from the agent")).unwrap();
    assert!(line.contains("codex-proj"), "{read}");
}

#[test]
fn a_command_before_any_hook_ran_gets_the_same_member_the_hook_would() {
    let h = Home::new();
    call(&h, &["--codex", "join", "lobby"], Input::Thread(THREAD));
    assert_eq!(members(&h).len(), 1);
    let o = start(&h, THREAD, &h.0.to_string_lossy(), "startup");
    assert!(o.status.success());
    assert_eq!(members(&h).len(), 1, "{:?}", members(&h));
}

#[test]
fn without_the_flag_nothing_changes_and_a_typed_name_always_wins() {
    let h = Home::new();
    let p = payload(THREAD, "/work/proj", "startup");
    call(&h, &["hook", "--event", "SessionStart"], Input::Hook(&p));
    assert!(members(&h).is_empty(), "no flag, no codex member");
    let o = call(&h, &["post", "lobby", "x"], Input::Thread(THREAD));
    assert!(
        !o.status.success(),
        "the thread id alone is not used without --codex"
    );
    // a typed name beats the flag
    let mut c = Command::new(env!("CARGO_BIN_EXE_klatch"));
    c.env_remove("KLATCH_IDENTITY_FILE");
    let o = c
        .env_clear()
        .env("KLATCH_HOME", &h.0)
        .env("KLATCH_KEY_DIR", h.0.join("keys"))
        .env("HOME", &h.0)
        .env("KLATCH_NAME", "typed")
        .env("CODEX_THREAD_ID", THREAD)
        .args(["--codex", "join", "lobby"])
        .output()
        .unwrap();
    assert!(o.status.success());
    let who = h.ok("observer", &["who", "lobby", "--all"]);
    assert!(who.contains("typed") && !who.contains("codex-"), "{who}");
}

#[test]
fn a_session_id_that_could_carry_a_path_or_a_command_is_ignored() {
    let h = Home::new();
    for bad in ["../../etc", "a;rm -rf ~", "x y", ""] {
        let p = payload(bad, "/work/proj", "startup");
        let o = call(
            &h,
            &["--codex", "hook", "--event", "SessionStart"],
            Input::Hook(&p),
        );
        assert!(o.status.success(), "a hook never fails a session");
    }
    assert!(members(&h).is_empty(), "{:?}", members(&h));
    let o = call(&h, &["--codex", "join", "lobby"], Input::Thread("bad id;x"));
    assert!(!o.status.success());
}
