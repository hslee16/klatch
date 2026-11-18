//! `klatch refusals`: a member lists its own recorded refusals at the command line (#678).
mod common;
use common::*;

const TOKEN: &str = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";

#[test]
fn a_member_with_no_refusal_is_told_so() {
    let h = Home::new();
    let out = h.ok("alice", &["refusals"]);
    assert!(out.contains("no refusals"), "{out}");
}

#[test]
fn a_member_sees_its_own_refusal_with_rule_room_and_operation_and_never_the_text() {
    let h = Home::new();
    h.fail("alice", &["post", "lobby", &format!("key {TOKEN}")]);
    let out = h.ok("alice", &["refusals"]);
    assert!(
        out.contains("lobby") && out.contains("post") && out.contains("credentials"),
        "{out}"
    );
    assert!(!out.contains(TOKEN) && !out.contains("abcdefgh"), "{out}");
}

#[test]
fn a_member_never_sees_another_members_refusal() {
    let h = Home::new();
    h.fail("bob", &["post", "lobby", &format!("key {TOKEN}")]);
    let out = h.ok("alice", &["refusals"]);
    assert!(out.contains("no refusals"), "{out}");
    assert!(
        !out.contains("bob") && !out.contains("credentials"),
        "{out}"
    );
}

#[test]
fn the_limit_flag_keeps_the_newest() {
    let h = Home::new();
    h.fail("alice", &["post", "lobby", &format!("key {TOKEN}")]);
    h.fail("alice", &["post", "--label", "internal", "lobby", "hello"]);
    let one = h.ok("alice", &["refusals", "--limit", "1"]);
    assert_eq!(one.lines().count(), 1, "{one}");
    assert!(one.contains("label"), "{one}");
    let all = h.ok("alice", &["refusals"]);
    assert_eq!(all.lines().count(), 2, "{all}");
}
