//! `klatch history ROOM [--last N] [--since SEQ]` (#518): read only, in the format `read` prints.
mod common;

use common::*;

fn busy_room(n: usize) -> Home {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    for i in 1..=n {
        h.ok("bob", &["post", "r", &format!("m{i}")]);
    }
    h
}

#[test]
fn history_prints_the_last_messages_in_the_read_format_and_the_data_rule() {
    let h = busy_room(5);
    let o = h.run("alice", &["history", "r", "--last", "2"]);
    let out = out(&o);
    assert!(
        out.contains("bob (unverified)] m4") && out.contains("bob (unverified)] m5"),
        "{out}"
    );
    assert!(!out.contains("m3"), "{out}");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("data from other agents"),
        "no data rule on stderr: {err}"
    );
    for line in out.lines() {
        assert!(
            line.starts_with('['),
            "a line that is not a message row: {line}"
        );
    }
}

#[test]
fn history_never_moves_the_read_cursor() {
    let h = busy_room(3);
    h.ok("alice", &["history", "r"]);
    h.ok("alice", &["history", "r", "--last", "1"]);
    let unread = h.ok("alice", &["read", "r"]);
    assert!(unread.contains("m1") && unread.contains("m3"), "{unread}");
}

#[test]
fn since_pages_forward_and_a_cut_says_how_many_more() {
    let h = busy_room(30);
    let first = h.ok("alice", &["history", "r", "--since", "2", "--last", "100"]);
    assert!(
        first.contains("] m3\n") && first.contains("] m30"),
        "{first}"
    );
    let page = h.ok("alice", &["history", "r", "--since", "2"]);
    assert!(
        page.contains("[8 more messages not shown; run `klatch history r --since 22`]"),
        "{page}"
    );
}

#[test]
fn a_flood_of_long_messages_is_cut_by_bytes_with_a_pointer_to_the_rest() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    for i in 0..10 {
        h.ok("bob", &["post", "r", &format!("{i} {}", "x".repeat(5000))]);
    }
    let out = h.ok("alice", &["history", "r", "--last", "10"]);
    assert!(out.len() < 20_000, "{} bytes", out.len());
    assert!(
        out.contains("more messages not shown; run `klatch history r --since"),
        "{out}"
    );
}

#[test]
fn a_bad_range_is_refused() {
    let h = busy_room(1);
    assert!(
        h.fail("alice", &["history", "r", "--last", "0"])
            .contains("bad history range")
    );
}
