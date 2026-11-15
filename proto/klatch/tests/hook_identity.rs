//! End-to-end tests: run the real binary against a throwaway KLATCH_HOME.
mod common;

use common::*;

#[test]
fn session_end_finds_its_member_by_the_remembered_session_id_when_the_record_is_gone() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    let start = hook_as_session(
        &h,
        Some("alice"),
        "sess-1",
        &["hook", "--event", "SessionStart"],
    );
    assert!(
        start.status.success(),
        "{}",
        String::from_utf8_lossy(&start.stderr)
    );
    // The harness has dropped its record and the hook has no typed name: only the session id is left.
    let end = hook_as_session(
        &h,
        None,
        "sess-1",
        &["--discover", "hook", "--event", "SessionEnd"],
    );
    assert!(
        end.status.success(),
        "{}",
        String::from_utf8_lossy(&end.stderr)
    );
    let who = h.ok("bob", &["who", "lobby", "--all"]);
    let line = who.lines().find(|l| l.starts_with("alice")).unwrap();
    assert!(line.contains("status: done"), "{who}");
}

#[test]
fn session_end_for_a_session_nobody_remembers_changes_nothing_and_does_not_fail() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    let end = hook_as_session(
        &h,
        None,
        "never-seen",
        &["--discover", "hook", "--event", "SessionEnd"],
    );
    assert!(
        end.status.success(),
        "a SessionEnd hook must never fail: {}",
        String::from_utf8_lossy(&end.stderr)
    );
    let who = h.ok("bob", &["who", "lobby", "--all"]);
    assert!(
        !who.contains("status: done"),
        "someone was marked done: {who}"
    );
}

fn hook_payload(agent: Option<&str>) -> String {
    let agent = agent.map_or(String::new(), |a| format!(r#","agent_id":"{a}""#));
    format!(r#"{{"session_id":"sess-9","hook_event_name":"PostToolUse"{agent}}}"#)
}

#[test]
fn a_hook_inside_a_subagent_prints_nothing_and_leaves_the_parents_mail_unread() {
    let h = Home::new();
    h.ok("parent", &["join", "lobby"]);
    h.ok("bob", &["join", "lobby"]);
    let args = ["hook", "--event", "PostToolUse"];
    hook_with_payload(&h, Some("parent"), &hook_payload(None), &args);
    // A subagent posts under its own typed name; its tool call fires the hook as the parent session.
    h.ok("bob", &["post", "lobby", "for the parent"]);
    let sub = hook_with_payload(&h, Some("parent"), &hook_payload(Some("agent-1")), &args);
    assert!(sub.status.success());
    assert_eq!(out(&sub).trim(), "", "a subagent's hook must stay quiet");
    // The parent's own next hook still delivers what the subagent's tool call did not consume.
    let parent = hook_with_payload(&h, Some("parent"), &hook_payload(None), &args);
    assert!(out(&parent).contains("for the parent"), "{}", out(&parent));
}
