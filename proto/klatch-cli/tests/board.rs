//! End-to-end tests: run the real binary against a throwaway KLATCH_HOME.
mod common;

use common::*;

#[test]
fn asks_lists_open_asks_with_their_state_and_a_release_does_not_end_one() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    h.ok("alice", &["post", "r", "seeking: a lib"]);
    h.ok("alice", &["post", "r", "chatter"]);
    let out = h.ok("bob", &["asks", "r"]);
    assert!(
        out.contains("#1") && out.contains("alice") && out.contains("unclaimed"),
        "got: {out}"
    );
    assert!(out.contains("seeking: a lib") && !out.contains("chatter"));
    h.ok("bob", &["claim", "r", "ask:1"]);
    let claimed = h.ok("bob", &["asks", "r"]);
    assert!(
        claimed.contains(" claimed ") && !claimed.contains("unclaimed"),
        "claimed state not shown: {claimed}"
    );
    h.ok("bob", &["release", "r", "ask:1"]);
    let after = h.ok("bob", &["asks", "r"]);
    assert!(
        after.contains("#1") && after.contains("unclaimed"),
        "a release ended the ask: {after}"
    );
    h.ok("bob", &["post", "r", "answer to #1: here"]);
    assert!(
        !h.ok("bob", &["asks", "r"]).contains("#1"),
        "an answered ask is still open"
    );
}

#[test]
fn board_shows_open_asks_offers_and_member_status() {
    let h = Home::new();
    h.ok("lib", &["join", "lobby"]);
    h.ok("lib", &["post", "lobby", "offering: text helpers"]);
    h.ok("api", &["post", "lobby", "seeking: slugify"]);
    h.ok("api", &["post", "lobby", "status: busy"]);
    let out = h.ok("lib", &["board", "lobby"]);
    assert!(out.starts_with("[board lobby]"), "got: {out}");
    assert!(
        out.contains("api unclaimed") && out.contains("seeking: slugify"),
        "got: {out}"
    );
    assert!(out.contains("lib offers") && out.contains("offering: text helpers"));
    assert!(out.contains("api last seen") && out.contains("status: busy"));
    h.ok("lib", &["post", "lobby", "withdrawn: no longer"]);
    assert!(
        h.ok("api", &["board", "lobby"])
            .contains("no current offers")
    );
    h.ok("lib", &["post", "lobby", "answer to #2: here"]);
    assert!(h.ok("api", &["board", "lobby"]).contains("no open asks"));
}

#[test]
fn the_session_start_board_is_shown_only_when_asked_for_and_does_not_join() {
    let h = Home::new();
    h.ok("lib", &["join", "lobby"]);
    h.ok("lib", &["post", "lobby", "offering: text helpers"]);
    h.ok("api", &["post", "lobby", "seeking: slugify"]);
    h.ok("newcomer", &["join", "elsewhere"]);
    let none = h.ok(
        "newcomer",
        &["hook", "--event", "SessionStart", "--no-join"],
    );
    assert!(
        !none.contains("[board"),
        "board shown without consent: {none}"
    );
    let start = h.ok(
        "newcomer",
        &[
            "hook",
            "--event",
            "SessionStart",
            "--board",
            "lobby",
            "--no-join",
        ],
    );
    assert!(start.contains("[board lobby]"), "got: {start}");
    assert!(start.contains("seeking: slugify") && start.contains("offering: text helpers"));
    assert!(start.contains("DATA"), "no data warning: {start}");
    let who = h.ok("lib", &["who", "lobby"]);
    assert!(
        !who.contains("newcomer"),
        "reading the board joined the reader: {who}"
    );
    let later = h.ok(
        "newcomer",
        &["hook", "--event", "UserPromptSubmit", "--board", "lobby"],
    );
    assert!(
        !later.contains("[board"),
        "board outside session start: {later}"
    );
}

#[test]
fn status_is_set_shown_by_who_and_the_board_and_clears_the_members_offers() {
    let h = Home::new();
    h.ok("lib", &["join", "lobby"]);
    h.ok("lib", &["post", "lobby", "offering: text helpers"]);
    h.ok("api", &["join", "lobby"]);
    assert!(h.ok("api", &["board", "lobby"]).contains("lib offers"));
    h.ok("lib", &["status", "lobby", "done", "leaving"]);
    let who = h.ok("api", &["who", "lobby"]);
    let line = who.lines().find(|l| l.starts_with("lib")).unwrap();
    assert!(line.contains("status: done leaving"), "got: {who}");
    let board = h.ok("api", &["board", "lobby"]);
    assert!(
        board.contains("lib last seen") && board.contains("status: done leaving"),
        "got: {board}"
    );
    assert!(board.contains("no current offers"), "offer stayed: {board}");
    let err = h.fail("api", &["status", "lobby", ""]);
    assert!(err.to_lowercase().contains("status"), "got: {err}");
}

#[test]
fn declines_show_on_asks_and_the_asker_is_told_once_when_nobody_can() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    h.ok("alice", &["post", "r", "seeking: a rare thing"]);
    h.ok("h1", &["post", "r", "declined #1 cannot: not mine"]);
    assert!(h.ok("alice", &["asks", "r"]).contains("[cannot 1]"));
    assert!(!h.ok("alice", &["hook"]).contains("nobody here may be able"));
    h.ok("h2", &["post", "r", "declined #1 cannot: not mine either"]);
    let asks = h.ok("alice", &["asks", "r"]);
    assert!(
        asks.contains("[cannot 2]") && asks.contains("#1"),
        "an ask with declines must stay open: {asks}"
    );
    let first = h.ok("alice", &["hook"]);
    assert!(
        first.contains("your ask #1: nobody here may be able to do this"),
        "got: {first}"
    );
    assert!(
        !h.ok("alice", &["hook"]).contains("nobody here may be able"),
        "told twice"
    );
}
