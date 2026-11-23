//! A session start prints the digest, the convention and the briefing (#1409, which replaced the board dump); together
//! they must stay inside the hook byte budget (#557), because Claude Code keeps only a 2 KB preview of hook output of
//! about 9.8 KB or more.
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
    let at = out.find("[briefing lobby]").expect("the briefing is kept");
    assert!(out.len() - at <= 2000, "the briefing is bounded: {out}");
    assert!(
        out.contains("open asks: 40"),
        "the asks are counted whole: {out}"
    );
    assert!(
        out.contains("klatch asks lobby"),
        "the way to read the rest: {out}"
    );
}

#[test]
fn a_crowded_room_is_counted_in_a_member_line_and_a_since_line_not_dumped() {
    // Many members and many long unaddressed messages: the briefing counts them, it prints none of the messages.
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
    assert!(!out.contains("note 00"), "the backlog was printed: {out}");
    assert!(
        out.contains("since you last read: 15 new in lobby, 0 for you"),
        "{out}"
    );
    assert!(out.contains("members: ") && out.contains("+"), "{out}");
}

#[test]
fn a_small_room_is_briefed_whole_with_nothing_cut() {
    let h = Home::new();
    h.ok("reader", &["join", "lobby"]);
    h.ok("peer", &["post", "lobby", "seeking: slugify"]);
    let out = start(&h);
    assert!(out.contains("#1 ask peer"), "{out}");
    assert!(out.contains("open asks: 1"), "{out}");
    assert!(!out.contains("more board lines"), "{out}");
}
