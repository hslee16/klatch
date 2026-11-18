//! End-to-end tests (#251): `klatch card ROOM [MEMBER]` shows one member's asks, work in hand and claims.
mod common;

use common::*;

fn room_with_an_ask_and_a_helper() -> Home {
    let h = Home::new();
    h.ok("alice", &["post", "lobby", "seeking: a slugify helper"]);
    h.ok("bob", &["join", "lobby"]);
    h.ok("bob", &["claim", "lobby", "ask:1"]);
    h.ok(
        "bob",
        &["claim", "lobby", "src/lib.rs", "--reason", "editing"],
    );
    h
}

#[test]
fn the_requester_sees_its_ask_and_its_state_on_its_own_card() {
    let h = room_with_an_ask_and_a_helper();
    let card = h.ok("alice", &["card", "lobby"]);
    assert!(card.starts_with("[card lobby alice]"), "{card}");
    assert!(card.contains("asks posted:"), "{card}");
    assert!(card.contains("#1 claimed"), "{card}");
    assert!(!card.contains("serving:"), "{card}");
}

#[test]
fn the_helper_card_shows_the_ask_it_serves_and_its_claims() {
    let h = room_with_an_ask_and_a_helper();
    let card = h.ok("alice", &["card", "lobby", "bob"]);
    assert!(card.starts_with("[card lobby bob]"), "{card}");
    assert!(card.contains("serving:"), "{card}");
    assert!(card.contains("#1 asked by alice"), "{card}");
    assert!(card.contains("claims:"), "{card}");
    assert!(card.contains("src/lib.rs"), "{card}");
    assert!(card.contains("DATA"), "{card}");
}

#[test]
fn a_name_that_is_not_on_the_board_has_no_card_and_the_command_fails() {
    let h = room_with_an_ask_and_a_helper();
    let err = h.fail("alice", &["card", "lobby", "nobody"]);
    assert!(err.contains("no card"), "{err}");
    let bad = h.fail("alice", &["card", "lobby", "bad name"]);
    assert!(bad.to_lowercase().contains("member"), "{bad}");
}
