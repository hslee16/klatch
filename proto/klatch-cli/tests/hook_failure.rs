//! A hook that cannot read the rooms says so in the agent's context (R19, #34): never silence, never a failure that
//! only the human sees.
mod common;

use common::*;

fn corrupt(h: &Home) {
    std::fs::write(
        h.0.join("rooms.db"),
        b"this is not a database at all, just bytes",
    )
    .unwrap();
}

fn context(out: &str) -> (String, serde_json::Value) {
    let v: serde_json::Value = serde_json::from_str(out.trim()).expect("one JSON object");
    (
        v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        v,
    )
}

#[test]
fn a_hook_that_cannot_open_the_store_says_so_in_the_context_and_exits_zero() {
    let h = Home::new();
    corrupt(&h);
    for event in ["SessionStart", "UserPromptSubmit", "PostToolUse"] {
        let o = h.run("alice", &["hook", "--event", event]);
        let out = String::from_utf8_lossy(&o.stdout).into_owned();
        assert!(o.status.success(), "{event} must not fail the session");
        let (ctx, v) = context(&out);
        assert!(
            ctx.starts_with("[klatch] could not read the rooms"),
            "{event}: {ctx}"
        );
        assert!(ctx.contains("messages may be waiting"), "{event}: {ctx}");
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], event);
        assert!(v["systemMessage"].is_string(), "the human is told too");
        assert_eq!(ctx.lines().count(), 1, "one line only: {ctx}");
    }
}

#[test]
fn the_failure_line_does_not_carry_the_whole_error_chain_or_a_newline() {
    let h = Home::new();
    corrupt(&h);
    let out = h.ok("alice", &["hook", "--event", "UserPromptSubmit"]);
    let (ctx, _) = context(&out);
    assert!(ctx.chars().count() < 300, "too long: {ctx}");
}

#[test]
fn a_hook_without_an_event_says_so_in_plain_text() {
    let h = Home::new();
    corrupt(&h);
    let out = h.ok("alice", &["hook"]);
    assert!(
        out.starts_with("[klatch] could not read the rooms"),
        "{out}"
    );
}

#[test]
fn session_end_stays_silent_when_the_store_is_broken() {
    let h = Home::new();
    corrupt(&h);
    let o = h.run("alice", &["hook", "--event", "SessionEnd"]);
    assert!(o.status.success());
    assert!(o.stdout.is_empty());
}

#[test]
fn a_command_that_is_not_a_hook_still_fails_loudly_on_a_broken_store() {
    let h = Home::new();
    corrupt(&h);
    let err = h.fail("alice", &["read", "lobby"]);
    assert!(!err.is_empty());
}

#[test]
fn a_healthy_store_prints_no_failure_line() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    assert_eq!(h.ok("alice", &["hook", "--event", "PostToolUse"]), "");
}
