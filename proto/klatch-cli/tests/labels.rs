//! Labels at the command line (epic #9). The admin acts need a terminal, so their effects are covered by the
//! gateway tests and the bench; here is what the binary does for an agent.
mod common;
use common::*;

#[test]
fn a_post_label_the_chain_does_not_have_is_refused_with_the_levels() {
    let h = Home::new();
    let err = h.fail(
        "alice",
        &["post", "--label", "top-secret", "lobby", "hello"],
    );
    assert!(
        err.contains("unknown label") && err.contains("public"),
        "{err}"
    );
}

#[test]
fn a_post_under_the_room_label_is_accepted() {
    let h = Home::new();
    h.ok("alice", &["post", "--label", "public", "lobby", "hello"]);
    assert!(h.ok("bob", &["read", "lobby"]).contains("hello"));
}

#[test]
fn a_post_labelled_above_an_unlabelled_room_is_refused() {
    let h = Home::new();
    let err = h.fail("alice", &["post", "--label", "internal", "lobby", "hello"]);
    assert!(err.contains("cannot go into room lobby"), "{err}");
}

#[test]
fn the_label_commands_need_a_real_terminal_like_every_admin_command() {
    let h = Home::new();
    for args in [
        vec!["admin", "label", "lobby", "private"],
        vec!["admin", "clear", "alice", "private"],
        vec!["admin", "cap", "internal"],
        vec!["admin", "chain", "a,b"],
        vec!["admin", "refusals"],
    ] {
        let err = h.fail("alice", &args);
        assert!(
            err.contains("real terminal") || err.contains("coding-agent session"),
            "{args:?}: {err}"
        );
    }
}

#[test]
fn the_wire_post_takes_a_label_and_refuses_one_above_the_room() {
    use klatch_protocol::ServerFrame;
    let h = Home::new();
    let f = h.serve(
        "alice",
        &[
            &request("a", "post", r#"{"room":"r","body":"ok","label":"public"}"#),
            &request("b", "post", r#"{"room":"r","body":"no","label":"private"}"#),
        ],
    );
    let ok = |id: &str| {
        f.iter()
            .find_map(|x| match x {
                ServerFrame::Response { id: i, ok, .. } if i == id => Some(*ok),
                _ => None,
            })
            .unwrap()
    };
    assert!(ok("a"));
    assert!(!ok("b"));
}
