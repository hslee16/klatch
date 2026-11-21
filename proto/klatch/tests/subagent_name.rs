//! The PreToolUse hook refuses an unnamed klatch command from a subagent (#1152). A subagent the harness runs inside the
//! session's own process shows no agent id to its commands, but its hook input carries `agent_id`; fake hook inputs
//! stand in for a real session here.
mod common;

use common::*;

/// A PreToolUse input for a tool call, with the `agent_id` Claude Code adds inside a subagent.
fn input(agent: Option<&str>, tool: &str, tool_input: serde_json::Value) -> String {
    let mut v = serde_json::json!({
        "session_id": "s-1",
        "hook_event_name": "PreToolUse",
        "tool_name": tool,
        "tool_input": tool_input,
        "cwd": "/w",
    });
    if let Some(a) = agent {
        v["agent_id"] = a.into();
        v["agent_type"] = "general-purpose".into();
    }
    v.to_string()
}

fn bash(agent: Option<&str>, command: &str) -> String {
    input(agent, "Bash", serde_json::json!({ "command": command }))
}

fn pre(h: &Home, stdin: &str) -> std::process::Output {
    h.run_stdin("alice", &["hook", "--event", "PreToolUse"], stdin)
}

fn home() -> Home {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    h
}

#[test]
fn a_subagent_klatch_command_without_a_name_is_denied_with_the_fix_on_stderr() {
    let h = home();
    let o = pre(&h, &bash(Some("a1"), "klatch post r hi"));
    let e = String::from_utf8_lossy(&o.stderr).into_owned();
    assert_eq!(o.status.code(), Some(2), "{e}");
    assert!(e.contains("KLATCH_NAME") && e.contains("subagent"), "{e}");
    assert!(!e.contains('\u{2014}'), "no em dash: {e}");
}

#[test]
fn a_subagent_klatch_command_that_names_itself_is_untouched() {
    let h = home();
    for c in [
        "KLATCH_NAME=t1-1 klatch post r hi",
        "klatch --as-member t1-1 post r hi",
    ] {
        let o = pre(&h, &bash(Some("a1"), c));
        assert!(o.status.success() && o.stdout.is_empty(), "{c}");
    }
}

#[test]
fn a_subagent_command_that_is_not_klatch_is_untouched() {
    let h = home();
    for c in ["cargo test", "grep klatch README.md", "ls proto/klatch"] {
        let o = pre(&h, &bash(Some("a1"), c));
        assert!(o.status.success() && o.stdout.is_empty(), "{c}");
    }
}

#[test]
fn the_parents_own_unnamed_klatch_command_is_untouched() {
    let h = home();
    let o = pre(&h, &bash(None, "klatch post r hi"));
    assert!(o.status.success() && o.stdout.is_empty());
    // an empty agent_id is not a subagent
    let o = pre(&h, &bash(Some(""), "klatch post r hi"));
    assert!(o.status.success());
}

#[test]
fn a_subagent_tool_call_that_is_not_bash_is_not_judged_by_the_name_rule() {
    let h = home();
    let w = input(
        Some("a1"),
        "Write",
        serde_json::json!({"file_path": "/w/notes.txt", "content": "klatch post r hi"}),
    );
    assert!(pre(&h, &w).status.success());
}
