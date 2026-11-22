//! `klatch ask ROOM TEXT --scope S` (#1171): the ask declares the scope it is about. It shows in the asks list, the
//! board and `read`; `could-answer` refuses a member whose own scope differs, and only then.
mod common;

use common::*;

fn number_in(out: &str) -> String {
    out.split('#')
        .nth(1)
        .and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next())
        .unwrap_or_else(|| panic!("no #N in {out}"))
        .to_string()
}

fn world() -> Home {
    let h = Home::new();
    for m in ["alice", "bob"] {
        h.ok(m, &["join", "lobby"]);
    }
    h
}

#[test]
fn a_declared_scope_shows_in_the_ask_the_asks_list_the_board_and_read() {
    let h = world();
    let said = h.ok(
        "alice",
        &["ask", "lobby", "fix", "the", "parser", "--scope", "git:foo"],
    );
    assert!(said.contains("scope git:foo"), "{said}");
    let asks = h.ok("bob", &["asks", "lobby"]);
    assert!(asks.contains("[scope git:foo]"), "{asks}");
    let board = h.ok("bob", &["board", "lobby"]);
    assert!(board.contains("scope git:foo"), "{board}");
    let read = h.ok("bob", &["read", "lobby", "--peek"]);
    assert!(
        read.contains("seeking: fix the parser [scope git:foo]"),
        "{read}"
    );
}

#[test]
fn an_ask_with_no_scope_shows_none_and_changes_nothing() {
    let h = world();
    h.ok("bob", &["scope", "git:bar"]);
    let n = number_in(&h.ok("alice", &["ask", "lobby", "plain"]));
    assert!(!h.ok("bob", &["asks", "lobby"]).contains("scope"));
    assert_eq!(h.ok("bob", &["could-answer", "lobby", &n]).trim(), "yes");
}

#[test]
fn could_answer_refuses_a_different_declared_scope_and_only_then() {
    let h = world();
    let n = number_in(&h.ok(
        "alice",
        &["ask", "lobby", "fix", "it", "--scope", "git:foo"],
    ));
    // a member with no scope is not affected
    assert_eq!(h.ok("bob", &["could-answer", "lobby", &n]).trim(), "yes");
    h.ok("bob", &["scope", "git:bar"]);
    let out = h.ok("bob", &["could-answer", "lobby", &n]);
    assert!(out.starts_with("no\nscope: "), "{out}");
    assert!(out.contains("git:foo") && out.contains("git:bar"), "{out}");
    h.ok("bob", &["scope", "git:foo"]);
    assert_eq!(h.ok("bob", &["could-answer", "lobby", &n]).trim(), "yes");
}

#[test]
fn a_bad_scope_and_a_scope_with_to_are_refused_and_nothing_is_posted() {
    let h = world();
    let bad = h.fail("alice", &["ask", "lobby", "x", "--scope", "has space"]);
    assert!(bad.to_lowercase().contains("scope"), "{bad}");
    let dm = h.fail("alice", &["ask", "--to", "bob", "x", "--scope", "git:foo"]);
    assert!(dm.contains("--scope"), "{dm}");
    assert!(h.ok("bob", &["asks", "lobby"]).trim().is_empty());
}
