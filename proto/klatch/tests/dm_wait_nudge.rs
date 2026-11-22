//! The dm nudge and the dm reply hint reach an agent that reads only through `klatch wait` and `klatch read` (#1209).
mod common;

use common::*;

const NUDGE_TAIL: &str = "for a longer exchange use: klatch dm bob";

/// A lobby of three where bob and alice alternate, ending on bob: five messages from the pair; carol only joined.
fn lobby_with_exchange(h: &Home) {
    h.ok("alice", &["join", "lobby"]);
    h.ok("carol", &["join", "lobby"]);
    h.ok("bob", &["post", "lobby", "b1"]);
    h.ok("alice", &["post", "lobby", "a1"]);
    h.ok("bob", &["post", "lobby", "b2"]);
    h.ok("alice", &["post", "lobby", "a2"]);
    h.ok("bob", &["post", "lobby", "b3"]);
}

fn count(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

#[test]
fn an_exchange_read_only_through_wait_gets_the_nudge_once() {
    let h = Home::new();
    lobby_with_exchange(&h);
    let first = h.ok("alice", &["wait", "lobby", "--timeout", "0"]);
    assert!(first.contains("b3"), "{first}");
    assert_eq!(count(&first, NUDGE_TAIL), 1, "{first}");
    assert!(
        first
            .lines()
            .last()
            .unwrap()
            .starts_with("klatch: you and bob exchanged "),
        "{first}"
    );
    // a later message of the same run does not repeat it
    h.ok("bob", &["post", "lobby", "b4"]);
    let second = h.ok("alice", &["wait", "lobby", "--timeout", "0"]);
    assert!(second.contains("b4"), "{second}");
    assert_eq!(count(&second, "for a longer exchange"), 0, "{second}");
}

#[test]
fn the_nudge_names_the_count_and_never_message_text() {
    let h = Home::new();
    lobby_with_exchange(&h);
    let out = h.ok("alice", &["wait", "lobby", "--timeout", "0"]);
    let nudge = out
        .lines()
        .find(|l| l.starts_with("klatch: you and"))
        .unwrap();
    assert_eq!(
        nudge,
        "klatch: you and bob exchanged 5 messages in lobby; for a longer exchange use: klatch dm bob \"<text>\""
    );
    assert!(!nudge.contains("b3") && !nudge.contains("a2"));
}

#[test]
fn read_gets_the_nudge_too_and_the_hook_then_does_not_repeat_it() {
    let h = Home::new();
    lobby_with_exchange(&h);
    let read = h.ok("alice", &["read", "lobby"]);
    assert_eq!(count(&read, NUDGE_TAIL), 1, "{read}");
    let hook = h.ok("alice", &["hook", "--event", "UserPromptSubmit"]);
    assert_eq!(count(&hook, "for a longer exchange"), 0, "{hook}");
}

#[test]
fn a_peek_leaves_the_nudge_for_the_hook() {
    let h = Home::new();
    lobby_with_exchange(&h);
    let peek = h.ok("alice", &["read", "lobby", "--peek"]);
    assert_eq!(count(&peek, "for a longer exchange"), 0, "{peek}");
    let hook = h.ok("alice", &["hook", "--event", "UserPromptSubmit"]);
    assert_eq!(count(&hook, NUDGE_TAIL), 1, "{hook}");
}

#[test]
fn no_nudge_in_a_room_of_two_or_once_a_dm_exists() {
    let h = Home::new();
    h.ok("alice", &["join", "pair"]);
    for i in 0..3 {
        h.ok("bob", &["post", "pair", &format!("b{i}")]);
        h.ok("alice", &["post", "pair", &format!("a{i}")]);
    }
    h.ok("bob", &["post", "pair", "last"]);
    let out = h.ok("alice", &["wait", "pair", "--timeout", "0"]);
    assert!(!out.contains("for a longer exchange"), "{out}");

    let h = Home::new();
    lobby_with_exchange(&h);
    h.ok("bob", &["dm", "alice", "taking this private"]);
    let out = h.ok("alice", &["wait", "lobby", "--timeout", "0"]);
    assert!(!out.contains("for a longer exchange"), "{out}");
}

#[test]
fn a_dm_message_returned_by_wait_carries_the_reply_hint() {
    let h = Home::new();
    h.ok("bob", &["dm", "alice", "SECRETBODY question"]);
    let out = h.ok("alice", &["wait", "dm:bob", "--timeout", "0"]);
    assert!(out.contains("SECRETBODY question"), "{out}");
    let hint = out.lines().last().unwrap();
    assert_eq!(
        hint,
        "klatch: dm from bob: reply with klatch dm bob \"<text>\""
    );
    h.ok("bob", &["dm", "alice", "second"]);
    let again = h.ok("alice", &["read", "dm.alice.bob"]);
    assert!(
        again
            .lines()
            .last()
            .unwrap()
            .starts_with("klatch: dm from bob: reply with"),
        "{again}"
    );
}

#[test]
fn nothing_is_added_when_nothing_was_returned() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    let out = h.ok("alice", &["wait", "lobby", "--timeout", "0"]);
    assert!(
        !out.contains("klatch: you and") && !out.contains("klatch: dm from"),
        "{out}"
    );
}
