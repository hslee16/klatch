//! The ask cycle note reaches an agent that never lets a wait time out and only runs hooks, `wait` that returns at
//! once and `read` (#789). The notes are derived from the room each time: they repeat while the cycle stands and stop
//! when a claim is released.
mod common;

use common::*;

/// alice asks #1, bob asks #2; bob holds alice's ask and alice holds bob's: a cycle.
fn cycle(h: &Home) {
    h.ok("alice", &["join", "lobby"]);
    h.ok("bob", &["join", "lobby"]);
    h.ok("alice", &["ask", "lobby", "alice needs a thing"]);
    h.ok("bob", &["ask", "lobby", "bob needs a thing"]);
    h.ok("bob", &["claim", "lobby", "ask:1"]);
    h.ok("alice", &["claim", "lobby", "ask:2"]);
}

fn hook(h: &Home, who: &str) -> String {
    let o = hook_as_session(h, Some(who), "s1", &["hook", "--event", "UserPromptSubmit"]);
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn a_member_that_only_runs_hooks_hears_the_cycle_at_every_hook_until_it_clears() {
    let h = Home::new();
    cycle(&h);
    let first = hook(&h, "alice");
    assert!(first.contains("ask cycle"), "{first}");
    let second = hook(&h, "alice");
    assert!(
        second.contains("ask cycle"),
        "repeats while it holds: {second}"
    );
    let note = first.split("klatch: ask cycle").nth(1).unwrap();
    assert!(
        !note.contains("needs a thing"),
        "never message text: {first}"
    );
    h.ok("alice", &["release", "lobby", "ask:2"]);
    let after = hook(&h, "alice");
    assert!(
        !after.contains("ask cycle"),
        "stops when it clears: {after}"
    );
}

#[test]
fn a_wait_says_the_cycle_once_and_a_read_carries_it_in_the_trailing_lines() {
    let h = Home::new();
    cycle(&h);
    h.ok("bob", &["post", "lobby", "hello"]);
    let waited = h.ok("alice", &["wait", "lobby", "--timeout", "5"]);
    assert!(waited.contains("hello"), "{waited}");
    // `wait` already says the cycle before it blocks; the trailing lines do not say it twice
    assert_eq!(waited.matches("ask cycle:").count(), 1, "{waited}");
    h.ok("bob", &["post", "lobby", "again"]);
    let read = h.ok("alice", &["read", "lobby"]);
    assert!(
        read.lines()
            .last()
            .unwrap()
            .starts_with("klatch: ask cycle:"),
        "{read}"
    );
    // a peek leaves the hook to say it
    h.ok("bob", &["post", "lobby", "third"]);
    let peek = h.ok("alice", &["read", "lobby", "--peek"]);
    assert!(!peek.contains("ask cycle"), "{peek}");
}
