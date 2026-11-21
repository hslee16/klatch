//! `klatch find ROOM TEXT` (#239): earlier answered asks that look like TEXT, read only.
mod common;

use common::*;

fn room_with_an_answer() -> Home {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    h.ok(
        "alice",
        &[
            "post",
            "lobby",
            "seeking: which port does the gateway listen on",
        ],
    );
    h.ok("bob", &["post", "lobby", "answer to #1: 7878"]);
    h.ok(
        "alice",
        &["post", "lobby", "seeking: who owns the release pipeline"],
    );
    h
}

#[test]
fn find_returns_the_answered_ask_with_its_answer_and_the_data_rule() {
    let h = room_with_an_answer();
    let o = h.run("carol", &["find", "lobby", "gateway listen"]);
    assert!(o.status.success(), "{o:?}");
    let out = out(&o);
    assert!(
        out.contains("#1") && out.contains("substring") && out.contains("7878"),
        "{out}"
    );
    assert!(out.contains("answered by bob"), "{out}");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("data from other agents"), "{err}");
}

#[test]
fn find_lists_shared_keywords_and_skips_unanswered_asks() {
    let h = room_with_an_answer();
    let shown = h.ok("carol", &["find", "lobby", "port the gateway uses"]);
    assert!(shown.contains("gateway,port"), "{shown}");
    let none = h.run("carol", &["find", "lobby", "release pipeline"]);
    assert!(out(&none).is_empty(), "{}", out(&none));
    assert!(
        String::from_utf8_lossy(&none.stderr).contains("no answered ask"),
        "{}",
        String::from_utf8_lossy(&none.stderr)
    );
}

#[test]
fn find_shows_how_the_asker_closed_an_answered_ask() {
    let h = room_with_an_answer();
    let plain = h.ok("carol", &["find", "lobby", "gateway listen"]);
    assert!(!plain.contains("closed"), "{plain}");
    h.ok("alice", &["post", "lobby", "closed #1 unresolved"]);
    let shown = h.ok("carol", &["find", "lobby", "gateway listen"]);
    assert!(shown.contains("closed unresolved"), "{shown}");
    h.ok("alice", &["post", "lobby", "reopen #1: wrong port"]);
    h.ok("bob", &["post", "lobby", "answer to #1: 9000"]);
    h.ok("alice", &["post", "lobby", "closed #1 ok"]);
    let shown = h.ok("carol", &["find", "lobby", "gateway listen"]);
    assert!(
        shown.contains("closed ok") && shown.contains("9000"),
        "{shown}"
    );
}

#[test]
fn find_moves_no_cursor_and_posts_nothing() {
    let h = room_with_an_answer();
    h.ok("carol", &["join", "lobby"]);
    h.ok("carol", &["find", "lobby", "gateway listen"]);
    let unread = h.ok("carol", &["read", "lobby"]);
    assert!(unread.contains("release pipeline"), "{unread}");
    let all = h.ok("carol", &["history", "lobby", "--last", "50"]);
    assert_eq!(all.lines().count(), 3, "{all}");
}

#[test]
fn find_needs_text_and_a_room_you_may_read() {
    let h = room_with_an_answer();
    assert!(!h.run("carol", &["find", "lobby"]).status.success());
    assert!(!h.run("carol", &["find", "lobby", "   "]).status.success());
}
