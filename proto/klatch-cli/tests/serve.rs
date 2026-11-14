//! End-to-end tests: run the real binary against a throwaway KLATCH_HOME.
mod common;

use common::*;
use klatch_protocol::ServerFrame;

#[test]
fn serve_stdio_says_hello_as_the_environments_member_and_only_prints_frames() {
    let h = Home::new();
    let frames = h.serve("alice", &[]);
    assert_eq!(frames.len(), 1);
    assert!(matches!(&frames[0], ServerFrame::Hello { member, .. } if member == "alice"));
}

#[test]
fn serve_stdio_answers_post_and_read_over_the_protocol() {
    let h = Home::new();
    let f = h.serve(
        "alice",
        &[&request(
            "p",
            "post",
            r#"{"room":"r","body":"over the wire"}"#,
        )],
    );
    assert!(
        f.iter()
            .any(|x| matches!(x, ServerFrame::Response { id, ok: true, .. } if id == "p"))
    );
    let f = h.serve("bob", &[&request("r", "read", r#"{"room":"r"}"#)]);
    let body = f.iter().find_map(|x| match x {
        ServerFrame::Response {
            id,
            result: Some(r),
            ..
        } if id == "r" => Some(r.to_string()),
        _ => None,
    });
    let body = body.expect("a read response");
    assert!(
        body.contains("over the wire") && body.contains("alice"),
        "{body}"
    );
}

#[test]
fn a_release_over_the_wire_carries_its_note_and_a_missing_note_still_works() {
    let h = Home::new();
    h.ok("bob", &["claim", "r", "a.rs"]);
    h.ok("bob", &["claim", "r", "b.rs"]);
    let f = h.serve(
        "bob",
        &[
            &request(
                "1",
                "release",
                r#"{"room":"r","path":"a.rs","note":"edited a"}"#,
            ),
            &request("2", "release", r#"{"room":"r","path":"b.rs"}"#),
        ],
    );
    assert_eq!(
        f.iter()
            .filter(|x| matches!(x, ServerFrame::Response { ok: true, .. }))
            .count(),
        2
    );
    let claimed = h.ok("carol", &["claim", "r", "a.rs"]);
    assert!(
        claimed.contains("last released by bob") && claimed.contains("edited a"),
        "{claimed}"
    );
}

#[test]
fn a_serve_session_shares_state_with_the_plain_cli() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    h.serve(
        "bob",
        &[&request(
            "p",
            "post",
            r#"{"room":"r","body":"from a session"}"#,
        )],
    );
    assert!(
        h.ok("alice", &["read", "r"])
            .contains("bob] from a session")
    );
}

#[test]
fn a_serve_request_cannot_post_as_someone_else() {
    let h = Home::new();
    h.serve(
        "alice",
        &[&request(
            "p",
            "post",
            r#"{"room":"r","body":"x","member":"bob","sender":"bob"}"#,
        )],
    );
    assert!(h.ok("carol", &["read", "r"]).contains("alice] x"));
}

#[test]
fn serve_without_stdio_is_refused_with_a_clear_message() {
    let h = Home::new();
    assert!(h.fail("alice", &["serve"]).contains("--stdio"));
}

#[test]
fn serve_stdio_still_requires_a_valid_member_name() {
    let h = Home::new();
    assert!(
        h.fail("bad name", &["serve", "--stdio"])
            .contains("KLATCH_NAME")
    );
}

#[test]
fn serve_stdio_keeps_going_after_a_bad_line() {
    let h = Home::new();
    let f = h.serve(
        "alice",
        &["garbage", &request("w", "who", r#"{"room":"r"}"#)],
    );
    assert!(
        f.iter()
            .any(|x| matches!(x, ServerFrame::Refusal { rule, .. } if rule == "protocol.decode"))
    );
    assert!(
        f.iter()
            .any(|x| matches!(x, ServerFrame::Response { id, ok: true, .. } if id == "w"))
    );
}
