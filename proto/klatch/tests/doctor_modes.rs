//! `klatch doctor` says which members run in bypassPermissions, because a socket nudge to such a member is held by
//! Claude Code (RESEARCH-NOTES 47.2, #553). The mode is learned from the `permission_mode` of a hook's input, the
//! only place Claude Code reports it. Doctor only reports; it never writes a setting.
mod common;

use common::*;
use std::process::Command;

fn hook_in_mode(h: &Home, name: &str, mode: Option<&str>, extra: &str) {
    let mode = mode.map_or(String::new(), |m| format!(r#","permission_mode":"{m}""#));
    let payload =
        format!(r#"{{"session_id":"s-{name}","hook_event_name":"PostToolUse"{mode}{extra}}}"#);
    let o = hook_with_payload(h, Some(name), &payload, &["hook", "--event", "PostToolUse"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

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

fn mode_lines(t: &str) -> Vec<&str> {
    t.lines()
        .filter(|l| l.contains("permission mode"))
        .collect()
}

#[test]
fn a_bypassing_member_is_warned_about_and_the_fix_never_writes_a_setting() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    hook_in_mode(&h, "alice", Some("bypassPermissions"), "");
    let t = doctor(&h);
    let l = mode_lines(&t);
    assert_eq!(l.len(), 1, "{t}");
    assert!(l[0].starts_with("warn") && l[0].contains("alice"), "{t}");
    assert!(l[0].contains("held"), "{t}");
    assert!(
        t.contains("hooks still deliver"),
        "the warning must say the hook path works: {t}"
    );
    assert!(
        !t.contains("crossSessionInbound: accept") || t.contains("only you can decide"),
        "doctor must not advise writing the setting without the human's decision: {t}"
    );
    assert!(!h.0.join(".claude").exists(), "doctor wrote settings");
}

#[test]
fn a_prompting_member_is_not_warned_about() {
    let h = Home::new();
    h.ok("bob", &["join", "lobby"]);
    hook_in_mode(&h, "bob", Some("default"), "");
    let t = doctor(&h);
    let l = mode_lines(&t);
    assert_eq!(l.len(), 1, "{t}");
    assert!(l[0].starts_with("ok") && l[0].contains("bob"), "{t}");
}

#[test]
fn an_unknown_mode_says_unknown_and_is_not_called_fine() {
    let h = Home::new();
    h.ok("carol", &["join", "lobby"]);
    hook_in_mode(&h, "carol", Some("somethingNew"), "");
    let t = doctor(&h);
    let l = mode_lines(&t);
    assert_eq!(l.len(), 1, "{t}");
    assert!(l[0].starts_with("warn") && l[0].contains("unknown"), "{t}");
}

#[test]
fn a_member_whose_hook_never_reported_a_mode_is_not_listed_as_fine() {
    let h = Home::new();
    h.ok("dave", &["join", "lobby"]);
    hook_in_mode(&h, "dave", None, "");
    let t = doctor(&h);
    assert!(
        mode_lines(&t).iter().all(|l| !l.contains("dave")),
        "a member with no recorded mode must not get an ok line: {t}"
    );
}

#[test]
fn the_last_reported_mode_wins_and_a_subagent_does_not_set_it() {
    let h = Home::new();
    h.ok("erin", &["join", "lobby"]);
    hook_in_mode(&h, "erin", Some("bypassPermissions"), "");
    hook_in_mode(&h, "erin", Some("default"), "");
    assert!(mode_lines(&doctor(&h))[0].starts_with("ok"));
    // a tool call inside a subagent carries the subagent's mode, which says nothing about this session
    hook_in_mode(
        &h,
        "erin",
        Some("bypassPermissions"),
        r#","agent_id":"sub-1""#,
    );
    assert!(mode_lines(&doctor(&h))[0].starts_with("ok"));
}
