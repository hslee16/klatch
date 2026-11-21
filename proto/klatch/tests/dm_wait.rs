//! `klatch dm` says how to wait for the reply, and `klatch wait` accepts `dm:NAME` (#1186).
mod common;

use common::*;
use std::time::{Duration, Instant};

#[test]
fn dm_output_ends_with_the_exact_wait_command_for_the_room() {
    let h = Home::new();
    let out = h.ok("alice", &["dm", "bob", "hello"]);
    assert!(
        out.contains("`klatch wait dm.alice.bob`"),
        "the dm output must name the wait command: {out}"
    );
    // Same room from the other side.
    let out = h.ok("bob", &["dm", "alice", "hi back"]);
    assert!(out.contains("`klatch wait dm.alice.bob`"), "got: {out}");
}

#[test]
fn dm_output_carries_no_message_text() {
    let h = Home::new();
    let out = h.ok("alice", &["dm", "bob", "SECRETBODY"]);
    assert!(!out.contains("SECRETBODY"), "got: {out}");
}

#[test]
fn wait_dm_name_reads_the_dm_room_with_that_member() {
    let h = Home::new();
    h.ok("alice", &["dm", "bob", "question"]);
    h.ok("bob", &["dm", "alice", "the answer"]);
    let out = h.ok("alice", &["wait", "dm:bob", "--timeout", "0"]);
    assert!(
        out.contains("[dm.alice.bob ") && out.contains("the answer"),
        "got: {out}"
    );
}

#[test]
fn wait_dm_name_mixes_with_other_rooms_and_takes_filters() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    h.ok("alice", &["dm", "bob", "question"]);
    h.ok("bob", &["dm", "alice", "ignored"]);
    h.ok("bob", &["dm", "alice", "seeking: wanted"]);
    h.ok("carol", &["post", "lobby", "chatter"]);
    let out = h.ok(
        "alice",
        &[
            "wait",
            "lobby:only=seeking:",
            "dm:bob:only=seeking:",
            "--timeout",
            "0",
        ],
    );
    assert!(
        out.contains("[dm.alice.bob ") && out.contains("wanted"),
        "got: {out}"
    );
    assert!(!out.contains("chatter"), "got: {out}");
}

#[test]
fn wait_dm_refuses_an_invalid_name_and_yourself() {
    let h = Home::new();
    assert!(
        h.fail("alice", &["wait", "dm:bad name", "--timeout", "0"])
            .contains("dm:")
    );
    assert!(
        h.fail("alice", &["wait", "dm:alice", "--timeout", "0"])
            .contains("yourself")
    );
    assert!(
        h.fail("alice", &["wait", "dm:", "--timeout", "0"])
            .contains("dm:")
    );
}

#[test]
fn wait_dm_with_nobody_there_yet_refuses_with_a_clear_error() {
    let h = Home::new();
    let err = h.fail("alice", &["wait", "dm:bob", "--timeout", "0"]);
    assert!(
        err.contains("dm:bob") || err.contains("dm.alice.bob"),
        "got: {err}"
    );
}

#[test]
fn a_plain_room_named_dm_with_a_filter_still_works() {
    let h = Home::new();
    h.ok("alice", &["join", "dm"]);
    h.ok("bob", &["post", "dm", "seeking: x"]);
    let out = h.ok("alice", &["wait", "dm:only=seeking:", "--timeout", "0"]);
    assert!(out.contains("seeking: x"), "got: {out}");
}

#[test]
fn a_dm_reply_wakes_a_wait_on_lobby_and_dm_name_at_once() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    h.ok("alice", &["dm", "bob", "question"]);
    let started = Instant::now();
    let child = h.spawn("alice", &["wait", "lobby", "dm:bob", "--timeout", "30"]);
    std::thread::sleep(Duration::from_millis(500));
    h.ok("bob", &["dm", "alice", "quick reply"]);
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("quick reply"), "got: {text}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the wait woke late: {:?}",
        started.elapsed()
    );
}
