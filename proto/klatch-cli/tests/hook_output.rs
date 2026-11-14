//! End-to-end tests: run the real binary against a throwaway KLATCH_HOME.
mod common;

use common::*;

#[test]
fn hook_with_event_prints_json_additional_context() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    h.ok("bob", &["post", "r", "ping \"quoted\""]);
    let out = h.ok("alice", &["hook", "--event", "PostToolUse"]);
    let v: serde_json::Value = serde_json::from_str(out.trim()).expect("one JSON object");
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PostToolUse");
    let ctx = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(ctx.contains("from bob: ping \"quoted\""), "got: {ctx}");
}

#[test]
fn hook_with_event_prints_nothing_when_there_is_nothing() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    assert_eq!(h.ok("alice", &["hook", "--event", "PostToolUse"]), "");
}

#[test]
fn a_message_the_hook_showed_is_still_readable_once() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    h.ok("bob", &["post", "r", "the ask"]);
    assert!(h.ok("alice", &["hook"]).contains("from bob: the ask"));
    assert!(h.ok("alice", &["read", "r"]).contains("the ask"));
    assert!(!h.ok("alice", &["read", "r"]).contains("the ask"));
}

#[test]
fn the_session_start_hook_lists_the_backlog_but_other_events_do_not() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    h.ok("alice", &["post", "lobby", "seeking: the api"]);
    h.ok("helper", &["join", "lobby"]);
    h.ok("helper", &["read", "lobby"]);
    let other = h.ok("helper", &["hook", "--event", "UserPromptSubmit"]);
    assert!(
        !other.contains("open ask"),
        "backlog outside session start: {other}"
    );
    let start = h.ok("helper", &["hook", "--event", "SessionStart"]);
    assert!(
        start.contains("open ask #1 from alice (no claim)"),
        "got: {start}"
    );
}
