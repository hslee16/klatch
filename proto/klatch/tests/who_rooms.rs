//! `klatch who` with no room lists every room the caller has joined (#227).
mod common;

use common::*;

#[test]
fn who_without_a_room_lists_the_members_of_every_joined_room() {
    let h = Home::new();
    h.ok("alice", &["join", "one"]);
    h.ok("alice", &["join", "two"]);
    h.ok("bob", &["join", "two"]);
    h.ok("carol", &["join", "three"]);
    let out = h.ok("alice", &["who"]);
    assert!(out.contains("one") && out.contains("two"), "got: {out}");
    assert!(out.contains("bob"), "got: {out}");
    assert!(
        !out.contains("three") && !out.contains("carol"),
        "got: {out}"
    );
}

#[test]
fn who_with_a_room_is_unchanged() {
    let h = Home::new();
    h.ok("alice", &["join", "one"]);
    h.ok("alice", &["join", "two"]);
    let out = h.ok("alice", &["who", "one"]);
    assert!(out.starts_with("alice\t"), "got: {out}");
    assert!(!out.contains("two"), "got: {out}");
}

#[test]
fn who_without_a_room_says_so_when_the_caller_has_joined_none() {
    let h = Home::new();
    h.ok("bob", &["join", "x"]);
    let out = h.ok("alice", &["who"]);
    assert!(out.contains("not joined any room"), "got: {out}");
}
