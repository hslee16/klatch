//! `board`, `asks`, `claims` and `history` with no room list every room the caller has joined, as `who` does (#734,
//! #227): a `[room]` header per room, never a room the caller did not join. They are the read-only commands that move no
//! cursor. `read` is left alone (it advances the caller's cursor, so a default over every room would consume messages the
//! caller did not ask to see) and `receipt` needs a room because its `seq` means nothing without one.
mod common;

use common::*;

/// bob has joined `one` and `two`; alice has posted and claimed in `one`, `two` and `three`, which bob has not joined.
fn world() -> Home {
    let h = Home::new();
    h.ok("bob", &["join", "one"]);
    h.ok("bob", &["join", "two"]);
    for room in ["one", "two", "three"] {
        h.ok("alice", &["join", room]);
        h.ok("alice", &["post", room, &format!("seeking: lib-{room}")]);
        h.ok("alice", &["claim", room, &format!("src/{room}.rs")]);
    }
    h
}

fn lists_only_the_joined_rooms(out: &str, marker: &str) {
    assert!(out.contains("[one]") && out.contains("[two]"), "got: {out}");
    assert!(
        out.contains(&format!("{marker}one")) && out.contains(&format!("{marker}two")),
        "got: {out}"
    );
    assert!(
        !out.contains("[three]") && !out.contains(&format!("{marker}three")),
        "a room bob did not join is listed: {out}"
    );
    assert!(
        out.find("[one]") < out.find("[two]"),
        "rooms come in the order `who` lists them: {out}"
    );
}

#[test]
fn asks_without_a_room_lists_the_open_asks_of_every_joined_room() {
    let out = world().ok("bob", &["asks"]);
    lists_only_the_joined_rooms(&out, "lib-");
}

#[test]
fn claims_without_a_room_lists_the_claims_of_every_joined_room() {
    let out = world().ok("bob", &["claims"]);
    lists_only_the_joined_rooms(&out, "src/");
}

#[test]
fn board_without_a_room_shows_the_board_of_every_joined_room() {
    let out = world().ok("bob", &["board"]);
    lists_only_the_joined_rooms(&out, "lib-");
}

#[test]
fn history_without_a_room_shows_the_newest_messages_of_every_joined_room() {
    let out = world().ok("bob", &["history"]);
    lists_only_the_joined_rooms(&out, "lib-");
}

#[test]
fn each_rooms_lines_sit_under_that_rooms_header() {
    let out = world().ok("bob", &["claims"]);
    let (one, two) = (out.find("[one]").unwrap(), out.find("[two]").unwrap());
    assert!(
        out[one..two].contains("src/one.rs") && !out[one..two].contains("src/two.rs"),
        "got: {out}"
    );
    assert!(out[two..].contains("src/two.rs"), "got: {out}");
}

#[test]
fn with_a_room_each_command_is_unchanged() {
    let h = world();
    for (cmd, mine, other) in [
        ("asks", "lib-one", "lib-two"),
        ("claims", "src/one.rs", "src/two.rs"),
        ("board", "lib-one", "lib-two"),
        ("history", "lib-one", "lib-two"),
    ] {
        let out = h.ok("bob", &[cmd, "one"]);
        assert!(out.contains(mine) && !out.contains(other), "{cmd}: {out}");
        assert!(
            !out.contains("[one]"),
            "{cmd} with a room prints no header: {out}"
        );
    }
}

#[test]
fn without_a_room_a_caller_that_joined_none_is_told_so() {
    let h = world();
    for cmd in ["asks", "claims", "board", "history"] {
        let out = h.ok("carol", &[cmd]);
        assert!(out.contains("not joined any room"), "{cmd}: {out}");
    }
}

#[test]
fn history_without_a_room_moves_no_cursor() {
    let h = world();
    h.ok("bob", &["history"]);
    let out = h.ok("bob", &["read", "one"]);
    assert!(out.contains("lib-one"), "history consumed it: {out}");
}

#[test]
fn history_since_without_a_room_is_refused_because_a_seq_belongs_to_one_room() {
    let h = world();
    let err = h.fail("bob", &["history", "--since", "1"]);
    assert!(err.contains("name the room"), "got: {err}");
}

#[test]
fn receipt_and_read_still_need_a_room() {
    let h = world();
    h.fail("bob", &["receipt"]);
    h.fail("bob", &["read"]);
}
