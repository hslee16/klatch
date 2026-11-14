//! End-to-end tests: run the real binary against a throwaway KLATCH_HOME.
mod common;

use common::*;

#[test]
fn an_ask_claim_defaults_to_a_short_lease_and_is_capped_and_other_claims_keep_theirs() {
    let h = Home::new();
    assert!(h.ok("a", &["claim", "r", "ask:1"]).contains("for 90s"));
    assert!(
        h.ok("a", &["claim", "r", "ask:2", "--ttl", "3600"])
            .contains("for 300s")
    );
    assert!(
        h.ok("a", &["claim", "r", "src/lib.rs"])
            .contains("for 600s")
    );
    assert!(
        h.ok("a", &["claim", "r", "ask:3", "--ttl", "30"])
            .contains("for 30s")
    );
}

#[test]
fn asks_show_a_question_waiting_on_the_asker_and_the_working_room() {
    let h = Home::new();
    h.ok("asker", &["join", "lobby"]);
    h.ok("asker", &["post", "lobby", "seeking: a thing"]);
    h.ok("helper", &["post", "lobby", "need-info #1: which format?"]);
    let waiting = h.ok("viewer", &["asks", "lobby"]);
    assert!(waiting.contains("needs info"), "got: {waiting}");
    h.ok("asker", &["post", "lobby", "info #1: json"]);
    assert!(
        !h.ok("viewer", &["asks", "lobby"]).contains("needs info"),
        "the answer ends the wait"
    );
    h.ok("asker", &["join", "ask-1"]);
    h.ok("helper", &["join", "ask-1"]);
    h.ok("third", &["join", "ask-1"]);
    let with_room = h.ok("viewer", &["asks", "lobby"]);
    assert!(
        with_room.contains("room ask-1 3 members"),
        "got: {with_room}"
    );
    assert!(
        h.ok("viewer", &["board", "lobby"])
            .contains("room ask-1 3 members")
    );
}

#[test]
fn a_hook_on_a_read_only_store_fails_open_with_one_visible_line() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    h.ok("bob", &["post", "lobby", "hello"]);
    read_only(&h, true);
    let o = h.run("alice", &["hook", "--event", "UserPromptSubmit"]);
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    let err = String::from_utf8_lossy(&o.stderr).into_owned();
    let end = h.run("alice", &["hook", "--event", "SessionEnd"]);
    read_only(&h, false);
    assert!(o.status.success(), "the hook must exit 0: {err}");
    assert!(
        out.contains("the store is read-only, so the lobby is unavailable"),
        "stdout: {out} stderr: {err}"
    );
    assert_eq!(out.lines().count(), 1, "one line only: {out}");
    assert!(end.status.success(), "SessionEnd must not fail either");
    assert!(end.stdout.is_empty(), "SessionEnd prints nothing");
    // nothing was changed, and a command that is not a hook still fails clearly
    assert!(h.ok("bob", &["read", "lobby", "--peek"]).is_empty());
}

#[test]
fn a_command_that_is_not_a_hook_still_fails_on_a_read_only_store() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    read_only(&h, true);
    let o = h.run("alice", &["post", "lobby", "x"]);
    read_only(&h, false);
    assert!(!o.status.success());
}
