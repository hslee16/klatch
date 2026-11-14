//! `klatch invite <member> <room>` (#133): a content-free pointer posted where the invited member already is. It
//! never joins anyone; the invited session decides.
mod common;

use common::*;

fn lobby() -> Home {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    h.ok("bob", &["join", "lobby"]);
    h
}

#[test]
fn an_invite_is_a_pointer_in_the_shared_room_that_mentions_the_member() {
    let h = lobby();
    h.ok("alice", &["invite", "bob", "design"]);
    let read = h.ok("bob", &["read", "lobby"]);
    assert!(
        read.contains("@bob") && read.contains("klatch join design"),
        "{read}"
    );
    assert!(read.contains("alice (unverified)"), "{read}");
}

#[test]
fn an_invite_never_joins_the_member_or_creates_a_membership() {
    let h = lobby();
    h.ok("alice", &["invite", "bob", "design"]);
    assert!(!h.ok("alice", &["who", "design"]).contains("bob"));
    assert!(!h.ok("bob", &["who", "design"]).contains("bob\t"));
}

#[test]
fn an_invite_carries_no_text_of_the_senders_and_nothing_but_the_pointer() {
    let h = lobby();
    h.ok("alice", &["invite", "bob", "design"]);
    let read = h.ok("bob", &["read", "lobby"]);
    let body = read.split("] ").last().unwrap().trim();
    assert_eq!(
        body,
        "invite: @bob, alice invites you to room design. To accept, run `klatch join design`; to ignore it, do nothing."
    );
}

#[test]
fn a_member_who_is_not_in_the_shared_room_cannot_be_reached_and_nothing_is_posted() {
    let h = lobby();
    let err = h.fail("alice", &["invite", "carol", "design"]);
    assert!(err.contains("carol") && err.contains("lobby"), "{err}");
    assert!(h.ok("bob", &["read", "lobby"]).is_empty());
}

#[test]
fn the_shared_room_can_be_named() {
    let h = Home::new();
    h.ok("alice", &["join", "team"]);
    h.ok("bob", &["join", "team"]);
    h.ok("alice", &["invite", "bob", "design", "--via", "team"]);
    assert!(
        h.ok("bob", &["read", "team"])
            .contains("klatch join design")
    );
}

#[test]
fn bad_names_are_refused_before_anything_is_posted() {
    let h = lobby();
    for args in [
        ["invite", "bob", "bad room"],
        ["invite", "bad name", "design"],
        ["invite", "alice", "design"],
    ] {
        assert!(!h.run("alice", &args).status.success(), "{args:?}");
    }
    assert!(h.ok("bob", &["read", "lobby"]).is_empty());
}

#[test]
fn an_invited_members_watcher_wakes_for_it_even_in_a_muted_room() {
    let h = lobby();
    h.ok("bob", &["mute", "lobby"]);
    h.ok("alice", &["invite", "bob", "design"]);
    assert_eq!(
        h.run("bob", &["wake", "--timeout", "5"]).status.code(),
        Some(2)
    );
}
