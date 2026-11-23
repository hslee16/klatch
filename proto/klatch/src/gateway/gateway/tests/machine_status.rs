//! The machine-derived member status through the gateway (#1403): what sets which status and when, what an explicit
//! status keeps, and that no message is ever posted.

use super::*;
use crate::domain::{MACHINE_STATUS_WINDOW_SECS, SESSION_END_STATUS};

fn status_of(g: &Gateway<SpyStore>, room: &str, who: &str) -> Option<String> {
    g.store
        .members(room)
        .unwrap()
        .into_iter()
        .find(|m| m.name == who)
        .and_then(|m| m.status)
}

#[test]
fn the_first_tool_use_sets_working_marked_as_machine_derived_and_posts_nothing() {
    let g = open();
    g.join(&alice(), "lobby").unwrap();
    let head = g.store.head("lobby").unwrap();
    assert_eq!(g.set_machine_status(&alice(), "working").unwrap(), 1);
    assert_eq!(
        status_of(&g, "lobby", "alice").as_deref(),
        Some("working (auto)")
    );
    assert_eq!(
        g.store.head("lobby").unwrap(),
        head,
        "no message was posted"
    );
    // the board and who show it
    g.join(&bob(), "lobby").unwrap();
    let board = g.board(&bob(), "lobby").unwrap();
    let a = board.members.iter().find(|m| m.name == "alice").unwrap();
    assert_eq!(a.status.as_deref(), Some("working (auto)"));
}

#[test]
fn a_burst_of_tool_calls_writes_once_and_the_next_write_comes_after_the_window() {
    let (g, t) = timed();
    g.join(&alice(), "lobby").unwrap();
    let mut written = 0;
    for _ in 0..20 {
        written += g.set_machine_status(&alice(), "working").unwrap();
        t.set(t.get() + 1);
    }
    assert_eq!(written, 1, "twenty calls in twenty seconds are one write");
    t.set(1_000 + MACHINE_STATUS_WINDOW_SECS);
    assert_eq!(g.set_machine_status(&alice(), "working").unwrap(), 1);
    assert_eq!(g.set_machine_status(&alice(), "working").unwrap(), 0);
}

#[test]
fn a_class_change_is_written_at_once_inside_the_window() {
    let g = open();
    g.join(&alice(), "lobby").unwrap();
    g.set_machine_status(&alice(), "working").unwrap();
    assert_eq!(g.set_machine_status(&alice(), "idle").unwrap(), 1);
    assert_eq!(
        status_of(&g, "lobby", "alice").as_deref(),
        Some("idle (auto)")
    );
    assert_eq!(g.set_machine_status(&alice(), "parked").unwrap(), 1);
    assert_eq!(
        status_of(&g, "lobby", "alice").as_deref(),
        Some("parked (auto)")
    );
    // and the next tool call after idle sets working again at once
    g.set_machine_status(&alice(), "idle").unwrap();
    assert_eq!(g.set_machine_status(&alice(), "working").unwrap(), 1);
}

#[test]
fn an_explicit_status_stays_through_tool_use_and_stop() {
    let g = open();
    g.join(&alice(), "lobby").unwrap();
    for explicit in [
        "parked: waiting on CI, back by 2h",
        "done",
        "working: parser",
        "blocked: x",
    ] {
        g.set_status(&alice(), "lobby", explicit).unwrap();
        for class in ["working", "idle", "parked"] {
            assert_eq!(
                g.set_machine_status(&alice(), class).unwrap(),
                0,
                "{explicit}"
            );
        }
        assert_eq!(status_of(&g, "lobby", "alice").as_deref(), Some(explicit));
    }
}

#[test]
fn once_the_member_changes_its_status_back_to_nothing_or_machine_text_the_machine_resumes() {
    let g = open();
    g.join(&alice(), "lobby").unwrap();
    g.set_status(&alice(), "lobby", "done").unwrap();
    // a post clears a done status (existing behaviour), and the machine may then write
    g.post(&alice(), "lobby", "back").unwrap();
    assert_eq!(g.set_machine_status(&alice(), "working").unwrap(), 1);
}

#[test]
fn a_session_end_status_is_the_machines_own_so_a_resumed_session_is_working_again() {
    let g = open();
    g.join(&alice(), "lobby").unwrap();
    g.set_status(&alice(), "lobby", SESSION_END_STATUS).unwrap();
    assert_eq!(g.set_machine_status(&alice(), "working").unwrap(), 1);
    assert_eq!(
        status_of(&g, "lobby", "alice").as_deref(),
        Some("working (auto)")
    );
}

#[test]
fn only_the_closed_class_words_are_taken() {
    let g = open();
    g.join(&alice(), "lobby").unwrap();
    for bad in ["done", "working (auto)", "Edit", "curl -H secret", ""] {
        assert!(is_invalid(g.set_machine_status(&alice(), bad)), "{bad:?}");
    }
    assert_eq!(status_of(&g, "lobby", "alice"), None);
}

#[test]
fn it_goes_to_every_joined_room_the_member_can_read_and_never_to_one_it_cannot() {
    let (g, t) = timed();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.join(&alice(), "vault").unwrap();
    g.join(&alice(), "open").unwrap();
    g.admin_set_clearance(&admin(), "alice", "public").unwrap();
    t.set(2_000);
    assert_eq!(g.set_machine_status(&alice(), "working").unwrap(), 1);
    assert_eq!(status_of(&g, "vault", "alice"), None);
    assert_eq!(
        status_of(&g, "open", "alice").as_deref(),
        Some("working (auto)")
    );
}

#[test]
fn a_member_that_is_in_no_room_gets_nothing_and_is_not_joined() {
    let g = open();
    assert_eq!(g.set_machine_status(&alice(), "working").unwrap(), 0);
    assert!(g.store.members("lobby").unwrap().is_empty());
}

#[test]
fn the_board_shows_the_members_own_status_message_over_the_machine_one() {
    let g = open();
    g.join(&alice(), "lobby").unwrap();
    g.join(&bob(), "lobby").unwrap();
    g.set_machine_status(&alice(), "working").unwrap();
    g.post(&alice(), "lobby", "status: reviewing the grammar")
        .unwrap();
    let board = g.board(&bob(), "lobby").unwrap();
    let a = board.members.iter().find(|m| m.name == "alice").unwrap();
    assert_eq!(a.status.as_deref(), Some("reviewing the grammar"));
}
