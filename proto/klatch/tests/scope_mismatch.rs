//! #894: a declared scope keeps winning over the repository derived from where a member runs, but a mismatch is made
//! visible: `who` shows one fixed marker and the member's own session start says so once. Only fixed words, never a
//! path or the other repository's name.
mod common;

use common::*;
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn repo(h: &Home) -> String {
    let dir = h.0.join("work").join("proj");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    dir.to_string_lossy().into_owned()
}

fn hook(h: &Home, scope: Option<&str>, event: &str, sid: &str, cwd: &str) -> Output {
    let payload = format!(
        r#"{{"session_id":"{sid}","cwd":"{cwd}","hook_event_name":"{event}","transcript_path":"/x"}}"#
    );
    let mut c = Command::new(env!("CARGO_BIN_EXE_klatch"));
    c.env_clear()
        .env("KLATCH_HOME", &h.0)
        .env("KLATCH_KEY_DIR", h.0.join("keys"))
        .env("HOME", &h.0)
        .current_dir(&h.0)
        .args(["--codex"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(s) = scope {
        c.args(["--member-scope", s]);
    }
    c.args(["hook", "--event", event]);
    let mut child = c.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

const SID: &str = "019a1b2c-3d4e-7f80-9a0b-1c2d3e4f5a6b";
const LINE: &str = "your declared scope differs from the repository you run in";

#[test]
fn a_declared_scope_that_differs_is_marked_in_who_and_said_once_at_session_start() {
    let h = Home::new();
    let cwd = repo(&h);
    let start = hook(
        &h,
        Some("git:elsewhere-00000001"),
        "SessionStart",
        SID,
        &cwd,
    );
    let text = out(&start);
    assert!(text.contains(LINE), "{text}");
    assert!(
        !text.contains("elsewhere"),
        "no foreign name echoed: {text}"
    );
    assert!(!text.contains(&cwd), "no path: {text}");
    let who = h.ok("observer", &["who", "lobby", "--all"]);
    let line = who.lines().find(|l| l.contains("codex-proj")).unwrap();
    assert!(line.contains("scope!=repo"), "{who}");
    assert!(!line.contains("elsewhere"), "{who}");
    // not on every hook: a later prompt hook does not repeat the line
    let later = hook(&h, None, "UserPromptSubmit", SID, &cwd);
    assert!(!out(&later).contains(LINE), "{}", out(&later));
}

#[test]
fn no_mismatch_no_marker_and_no_line() {
    let h = Home::new();
    let cwd = repo(&h);
    let start = hook(&h, None, "SessionStart", SID, &cwd);
    assert!(!out(&start).contains(LINE), "{}", out(&start));
    let who = h.ok("observer", &["who", "lobby", "--all"]);
    assert!(!who.contains("scope!=repo"), "{who}");
}
