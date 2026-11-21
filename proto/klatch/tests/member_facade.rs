//! `klatch::member` is usable from outside the crate the way `klatch-server` needs (#1062): the member, scope, home
//! and key directory come from the environment, the store is opened and keyed by the library, and a gateway for that
//! member posts and reads. Everything lives in a throwaway home with its own key directory; the live store and the
//! keychain are never touched. One test only, because it sets process environment variables.

#[path = "common/mod.rs"]
mod common;

use klatch::member::{Call, Who, open_member};

#[test]
fn the_facade_opens_a_member_from_the_environment_and_posts_and_reads() {
    let home = common::Home::new();
    // SAFETY: this file holds one test, so no other thread reads or writes the environment while these are set.
    unsafe {
        std::env::set_var("HOME", &home.0);
        std::env::set_var("KLATCH_HOME", &home.0);
        std::env::set_var("KLATCH_KEY_DIR", home.0.join("keys"));
        std::env::set_var("KLATCH_NAME", "facade-bot");
        std::env::set_var("KLATCH_SCOPE", "bot:facade");
        std::env::remove_var("KLATCH_IDENTITY_FILE");
        std::env::remove_var("KLATCH_DISCOVER");
    }

    let member = open_member(&Who::default(), Call::default())
        .expect("the member opens")
        .expect("a typed name is a caller");
    assert_eq!(member.principal().member, "facade-bot");

    member.gateway.join(member.principal(), "lobby").unwrap();
    member
        .gateway
        .post(member.principal(), "lobby", "hello from the facade")
        .unwrap();
    // a second member (the store is shared) reads what the first posted
    // SAFETY: as above, the only test in this file.
    unsafe {
        std::env::set_var("KLATCH_NAME", "facade-peer");
    }
    let peer = open_member(&Who::default(), Call::default())
        .expect("the peer opens")
        .expect("a typed name is a caller");
    peer.gateway.join(peer.principal(), "lobby").unwrap();
    let read = peer.gateway.read(peer.principal(), "lobby", false).unwrap();
    assert!(read.iter().any(|m| m.body == "hello from the facade"));
    assert!(
        home.0.join("rooms.db").exists(),
        "the store is in the temp home"
    );
}
