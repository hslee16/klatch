//! A muted room shows a member only the messages that mention it (#20).
mod common;

use common::*;

fn chatty(h: &Home) {
    h.ok("alice", &["join", "lobby"]);
    h.ok("bob", &["post", "lobby", "chatter one"]);
    h.ok("bob", &["post", "lobby", "@alice please look at this"]);
    h.ok("bob", &["post", "lobby", "chatter two"]);
}

#[test]
fn a_muted_room_shows_the_hook_only_what_mentions_the_member() {
    let h = Home::new();
    chatty(&h);
    let said = h.ok("alice", &["mute", "lobby"]);
    assert!(said.contains("muted lobby"), "{said}");
    let hook = h.ok("alice", &["hook", "--event", "UserPromptSubmit"]);
    assert!(hook.contains("@alice please look at this"), "{hook}");
    assert!(!hook.contains("chatter"), "{hook}");
}

#[test]
fn a_mute_leaves_read_alone_and_unmute_brings_everything_back() {
    let h = Home::new();
    chatty(&h);
    h.ok("alice", &["mute", "lobby"]);
    h.ok("alice", &["hook", "--event", "UserPromptSubmit"]);
    let read = h.ok("alice", &["read", "lobby", "--peek"]);
    assert!(
        read.contains("chatter one") && read.contains("chatter two"),
        "{read}"
    );
    h.ok("alice", &["unmute", "lobby"]);
    h.ok("bob", &["post", "lobby", "chatter three"]);
    let hook = h.ok("alice", &["hook", "--event", "UserPromptSubmit"]);
    assert!(hook.contains("chatter three"), "{hook}");
}

#[test]
fn muting_a_room_with_a_bad_name_is_refused() {
    let h = Home::new();
    assert!(
        h.fail("alice", &["mute", "bad room"])
            .contains("bad room name")
    );
}
