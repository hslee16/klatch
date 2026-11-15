//! A session start prints the digest, the convention and the boards; together they must stay inside the hook byte
//! budget (#557), because Claude Code keeps only a 2 KB preview of hook output of about 9.8 KB or more.
mod common;

use common::*;

const HOOK_CAP: usize = 9_700;

/// A room with a big board (many long asks) and many long unread messages for `reader`, who is already a member.
fn crowded() -> Home {
    let h = Home::new();
    h.ok("reader", &["join", "lobby"]);
    for i in 0..40 {
        h.ok(
            "peer",
            &[
                "post",
                "lobby",
                &format!("seeking: {i:02} {}", "q".repeat(150)),
            ],
        );
    }
    for i in 0..15 {
        h.ok(
            "peer",
            &["post", "lobby", &format!("note {i:02} {}", "z".repeat(900))],
        );
    }
    h
}

fn start(h: &Home) -> String {
    h.ok(
        "reader",
        &["hook", "--event", "SessionStart", "--board", "lobby"],
    )
}

#[test]
fn a_session_start_with_a_huge_board_and_many_messages_stays_inside_the_hook_budget() {
    let out = start(&crowded());
    assert!(out.len() < HOOK_CAP, "the hook printed {} bytes", out.len());
    assert!(out.contains("[board lobby]"), "the board header is kept");
    assert!(
        out.contains("more board lines not shown; run `klatch board lobby`"),
        "the cut is not counted: {out}"
    );
}

#[test]
fn the_board_shrinks_before_the_messages_do() {
    // Open asks are digest items that go before messages by design (item 50), and offers are messages, so build the
    // big board from members, which only the board lists.
    let h = Home::new();
    h.ok("reader", &["join", "lobby"]);
    for i in 0..25 {
        h.ok(&format!("{}{i:02}", "m".repeat(36)), &["join", "lobby"]);
    }
    for i in 0..15 {
        h.ok(
            "talker",
            &["post", "lobby", &format!("note {i:02} {}", "z".repeat(900))],
        );
    }
    let out = start(&h);
    assert!(out.len() < HOOK_CAP, "{} bytes", out.len());
    assert!(out.contains("more board lines not shown"), "{out}");
    let shown = out.matches("note ").count();
    assert!(shown >= 4, "only {shown} messages shown: {out}");
}

#[test]
fn a_small_board_is_shown_whole_with_no_cut_line() {
    let h = Home::new();
    h.ok("reader", &["join", "lobby"]);
    h.ok("peer", &["post", "lobby", "seeking: slugify"]);
    let out = start(&h);
    assert!(out.contains("seeking: slugify"), "{out}");
    assert!(!out.contains("more board lines"), "{out}");
}
