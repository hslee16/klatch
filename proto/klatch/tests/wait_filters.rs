//! `klatch wait` filters per room and a repeated `--only` (#219).
mod common;

use common::*;

const TIMEOUT: &str = "1";

#[test]
fn a_room_can_carry_its_own_only_filter_so_a_room_of_two_wakes_on_anything() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    h.ok("alice", &["join", "dm"]);
    h.ok("bob", &["post", "lobby", "chatter"]);
    h.ok("bob", &["post", "dm", "any reply"]);
    let out = h.ok(
        "alice",
        &["wait", "lobby:only=seeking:", "dm", "--timeout", "5"],
    );
    assert!(
        out.contains("[dm ") && out.contains("any reply"),
        "got: {out}"
    );
    assert!(!out.contains("chatter"), "got: {out}");
}

#[test]
fn a_room_filter_still_ignores_non_matching_messages_in_that_room() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    h.ok("alice", &["join", "dm"]);
    h.ok("bob", &["post", "lobby", "chatter"]);
    let out = h.ok(
        "alice",
        &["wait", "lobby:only=seeking:", "dm", "--timeout", TIMEOUT],
    );
    assert!(out.contains("[klatch wait: timeout"), "got: {out}");
    assert!(
        h.ok("alice", &["read", "lobby"]).contains("chatter"),
        "a filter lost a message"
    );
}

#[test]
fn a_room_with_its_own_filter_ignores_the_global_only() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    h.ok("alice", &["join", "dm"]);
    // the dm room has news that does not match the global filter; with the room's own filter the lobby message wakes
    // the wait (when both rooms match, the room with the more important message is shown first, #1405)
    h.ok("bob", &["post", "dm", "other news"]);
    h.ok("bob", &["post", "lobby", "closing: y"]);
    let out = h.ok(
        "alice",
        &[
            "wait",
            "lobby:only=closing:",
            "dm",
            "--only",
            "seeking:",
            "--timeout",
            "5",
        ],
    );
    assert!(out.contains("closing: y"), "got: {out}");
    assert!(!out.contains("other news"), "got: {out}");
}

#[test]
fn only_repeats_and_any_match_wakes() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    h.ok("bob", &["post", "r", "chatter"]);
    h.ok("bob", &["post", "r", "closing: done"]);
    let out = h.ok(
        "alice",
        &[
            "wait",
            "r",
            "--only",
            "seeking:",
            "--only",
            "closing:",
            "--timeout",
            "5",
        ],
    );
    assert!(
        out.contains("closing: done") && !out.contains("chatter"),
        "got: {out}"
    );
}

#[test]
fn the_same_room_named_twice_with_filters_wakes_on_either() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    h.ok("bob", &["post", "lobby", "closing: done"]);
    let out = h.ok(
        "alice",
        &[
            "wait",
            "lobby:only=seeking:",
            "lobby:only=closing:",
            "--timeout",
            "5",
        ],
    );
    assert!(out.contains("closing: done"), "got: {out}");
}

#[test]
fn a_room_filter_can_name_a_sender() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    h.ok("carol", &["post", "r", "noise"]);
    h.ok("bob", &["post", "r", "from bob"]);
    let out = h.ok("alice", &["wait", "r:from=bob", "--timeout", "5"]);
    assert!(
        out.contains("from bob") && !out.contains("noise"),
        "got: {out}"
    );
}

#[test]
fn a_bad_room_filter_is_refused_with_the_form() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    let msg = h.fail("alice", &["wait", "r:bogus=x", "--timeout", "0"]);
    assert!(msg.contains("only=") && msg.contains("from="), "got: {msg}");
}
