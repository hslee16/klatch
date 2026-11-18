//! `klatch grant ROOM MEMBER LABEL` (#893): the creator of a room is its grantor. The admin acts that name a grantor
//! or set a room grant need a terminal, so they are proved at the gateway (`gateway/tests/room_grantor.rs`); here the
//! command itself is run as two members, with a temporary home.
mod common;

use common::*;

#[test]
fn the_creator_grants_within_its_clearance_and_a_non_grantor_is_refused() {
    let h = Home::new();
    h.ok("alice", &["join", "design"]);
    h.ok("bob", &["join", "design"]);
    // the room is public: the grantor can grant public, which changes nothing, and the command says so
    let out = h.ok("alice", &["grant", "design", "bob", "public"]);
    assert!(out.contains("bob") && out.contains("design"), "{out}");
    // bob is not the grantor of the room alice made
    let err = h.fail("bob", &["grant", "design", "alice", "public"]);
    assert!(err.contains("grantor"), "{err}");
}

#[test]
fn a_grantor_cannot_grant_itself_or_the_top_level_and_nothing_is_stored() {
    let h = Home::new();
    h.ok("alice", &["join", "design"]);
    h.ok("bob", &["join", "design"]);
    let err = h.fail("alice", &["grant", "design", "alice", "public"]);
    assert!(err.contains("itself"), "{err}");
    let err = h.fail("alice", &["grant", "design", "bob", "secret"]);
    assert!(err.contains("top level"), "{err}");
    let err = h.fail("alice", &["grant", "design", "bob", "nonsense"]);
    assert!(err.contains("nonsense"), "{err}");
}

#[test]
fn a_member_cannot_grant_in_a_room_it_only_joined() {
    let h = Home::new();
    h.ok("alice", &["join", "design"]);
    h.ok("bob", &["join", "design"]);
    h.ok("bob", &["join", "mine"]);
    let err = h.fail("alice", &["grant", "mine", "bob", "public"]);
    assert!(err.contains("grantor"), "{err}");
}

#[test]
fn the_admin_acts_need_a_terminal_like_every_admin_command() {
    let h = Home::new();
    h.ok("alice", &["join", "design"]);
    for args in [
        vec!["admin", "grantor", "design", "bob"],
        vec!["admin", "room-clear", "design", "bob", "internal"],
        vec!["admin", "room-clear", "design", "bob", "--off"],
    ] {
        assert!(!h.run("alice", &args).status.success(), "{args:?}");
    }
}
