//! The conformance scenario for a klatch client (#119), in Rust: the same steps as the Python and TypeScript clients in
//! `clients/`, run against the real binary through `klatch::client`.
mod common;

use common::*;
use klatch::client::{ClaimRequest, Client, PostRequest, ReadRequest, RefInput};
use klatch::protocol::decode_server_line;
use std::path::PathBuf;
mod client_scenario;

/// What the hello says of a member here: a typed name is unattributed.
pub const ATTRIBUTION: &str = "unattributed";

pub fn connect(h: &Home, name: &str) -> Client {
    let env = vec![
        (
            "KLATCH_HOME".to_string(),
            h.0.to_string_lossy().into_owned(),
        ),
        (
            "KLATCH_KEY_DIR".to_string(),
            h.0.join("keys").to_string_lossy().into_owned(),
        ),
        ("KLATCH_NAME".to_string(), name.to_string()),
        (
            "PATH".to_string(),
            std::env::var("PATH").unwrap_or_default(),
        ),
    ];
    Client::connect(&PathBuf::from(env!("CARGO_BIN_EXE_klatch")), &env).unwrap()
}

pub fn home() -> Home {
    Home::new()
}

#[test]
fn every_fixture_line_is_understood_and_unknown_things_are_ignored() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/protocol");
    let server = std::fs::read_to_string(dir.join("server_basic.ndjson")).unwrap();
    let lines: Vec<&str> = server.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(lines.len() > 5);
    for l in &lines {
        decode_server_line(l).unwrap_or_else(|e| panic!("{l}: {e:?}"));
    }
    for l in std::fs::read_to_string(dir.join("forward_compat.ndjson"))
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
    {
        decode_server_line(l).unwrap();
    }
    for bad in [
        "not json",
        "[1]",
        r#"{"type":"message"}"#,
        r#"{"v":2,"type":"message"}"#,
    ] {
        assert!(decode_server_line(bad).is_err(), "{bad}");
    }
}

#[test]
fn a_request_builder_sets_only_what_it_is_given() {
    let post = PostRequest::new("r", "text");
    assert_eq!((post.room, post.body, post.label), ("r", "text", None));
    assert!(post.handoff.is_none());
    let post = PostRequest::new("r", "text").label("internal");
    assert_eq!(post.label, Some("internal"));
    let claim = ClaimRequest::new("r", "a.rs");
    assert_eq!((claim.ttl, claim.reason), (None, None));
    let claim = ClaimRequest::new("r", "a.rs").ttl(60).reason("why");
    assert_eq!((claim.ttl, claim.reason), (Some(60), Some("why")));
}

#[test]
fn a_post_request_carries_a_scope_only_when_given_one() {
    assert_eq!(PostRequest::new("r", "seeking: x").scope, None);
    assert_eq!(
        PostRequest::new("r", "seeking: x").scope("git:foo").scope,
        Some("git:foo")
    );
}

#[test]
fn a_post_request_carries_refs_only_when_given_some() {
    assert!(PostRequest::new("r", "seeking: x").refs.is_empty());
    let refs = [
        RefInput::new("pr", "o/r#1").label("fix"),
        RefInput::new("log", "ci.log").snippet("boom"),
    ];
    let req = PostRequest::new("r", "seeking: x").refs(&refs);
    assert_eq!(req.refs.len(), 2);
    assert_eq!(req.refs[0].label, Some("fix"));
    assert_eq!(req.refs[1].snippet, Some("boom"));
}

#[test]
fn a_read_request_carries_a_bound_only_when_given_one() {
    let req = ReadRequest::new("r");
    assert_eq!(
        (req.peek, req.max_messages, req.max_bytes),
        (false, None, None)
    );
    let req = ReadRequest::new("r")
        .peek(true)
        .max_messages(3)
        .max_bytes(2000);
    assert_eq!(
        (req.peek, req.max_messages, req.max_bytes),
        (true, Some(3), Some(2000))
    );
}

#[test]
fn a_claim_request_carries_shared_only_when_given_it() {
    assert_eq!(ClaimRequest::new("r", "a.rs").shared, None);
    assert_eq!(
        ClaimRequest::new("r", "a.rs").shared(true).shared,
        Some(true)
    );
    assert_eq!(
        ClaimRequest::new("r", "a.rs").shared(false).shared,
        Some(false)
    );
}

#[test]
fn a_bounded_read_leaves_the_rest_unread_and_says_how_many() {
    let h = home();
    let (mut a, mut b) = (connect(&h, "alice"), connect(&h, "bob"));
    for body in ["one", "two", "three"] {
        a.post(PostRequest::new("r", body)).unwrap();
    }
    let first = b
        .read_bounded(ReadRequest::new("r").max_messages(2))
        .unwrap();
    let bodies: Vec<_> = first.messages.iter().map(|m| m.body.as_str()).collect();
    assert_eq!((bodies, first.omitted), (vec!["one", "two"], Some(1)));
    let rest = b
        .read_bounded(ReadRequest::new("r").max_bytes(100_000))
        .unwrap();
    let bodies: Vec<_> = rest.messages.iter().map(|m| m.body.as_str()).collect();
    assert_eq!((bodies, rest.omitted), (vec!["three"], Some(0)));
}

#[test]
fn an_unbounded_read_request_has_no_omitted_and_a_peek_moves_nothing() {
    let h = home();
    let (mut a, mut b) = (connect(&h, "alice"), connect(&h, "bob"));
    a.post(PostRequest::new("r", "one")).unwrap();
    let peeked = b.read_bounded(ReadRequest::new("r").peek(true)).unwrap();
    assert_eq!((peeked.messages.len(), peeked.omitted), (1, None));
    assert_eq!(b.read("r", false).unwrap().len(), 1);
}

#[test]
fn a_bad_bound_is_the_servers_refusal_not_a_client_one() {
    let h = home();
    let mut a = connect(&h, "alice");
    a.post(PostRequest::new("r", "one")).unwrap();
    // the request types make a negative bound impossible; the server still refuses what it cannot take
    let err = a
        .request("read", serde_json::json!({"room": "r", "max_messages": -1}))
        .unwrap_err();
    assert!(matches!(err, klatch::client::Error::Api(_)), "{err}");
}

#[test]
fn a_shared_claim_is_listed_with_scope_star_and_an_exclusive_one_is_not() {
    let h = home();
    let mut a = connect(&h, "alice");
    a.claim(ClaimRequest::new("r", "docs/").shared(true))
        .unwrap();
    a.claim(ClaimRequest::new("r", "src/").shared(false))
        .unwrap();
    let scopes: Vec<_> = a
        .claims("r")
        .unwrap()
        .into_iter()
        .map(|c| (c.path, c.scope))
        .collect();
    assert_eq!(
        scopes,
        vec![
            ("docs/".to_string(), Some("*".to_string())),
            ("src/".to_string(), None)
        ]
    );
}

/// A store with a crossing waiting: `vault` is private (alice and the reviewer `rev` are cleared to it), `open` is
/// public, and alice has asked for her message to be copied from the one into the other. Returns the message's seq.
fn crossing_waiting(h: &Home) -> i64 {
    let mut alice = connect(h, "alice");
    for room in ["vault", "open"] {
        alice.post(PostRequest::new(room, "hello")).unwrap();
    }
    for act in [
        &["label", "vault", "private"][..],
        &["clear", "alice", "private"],
        &["clear", "rev", "private"],
        &["reviewer", "rev"],
    ] {
        let r = h.admin(act, &[]);
        assert!(r.ok(), "{act:?} failed:\n{}", r.text);
    }
    let (seq, _) = alice
        .post(PostRequest::new("vault", "the plan is blue"))
        .unwrap();
    let r = h.run(
        "alice",
        &["cross", "vault", &seq.to_string(), "--to", "open"],
    );
    assert!(r.status.success(), "{r:?}");
    seq as i64
}

#[test]
fn a_reviewer_gets_the_item_with_its_target_room_and_its_current_label_over_the_client() {
    let h = home();
    let seq = crossing_waiting(&h);
    let mut rev = connect(&h, "rev");
    let item = rev.review_next().unwrap().expect("one item waits");
    assert_eq!(
        (item.room.as_str(), item.seq as i64, item.text.as_str()),
        ("vault", seq, "the plan is blue")
    );
    assert_eq!(
        (item.to_room.as_str(), item.to_label.as_str()),
        ("open", "public")
    );
    // the target is relabelled after the request was queued: the reviewer sees the label a release will use (#1767)
    let r = h.admin(&["label", "open", "internal"], &[]);
    assert!(r.ok(), "{}", r.text);
    let item = rev.review_next().unwrap().expect("still waiting");
    assert_eq!(
        (item.to_room.as_str(), item.to_label.as_str()),
        ("open", "internal")
    );
}

#[test]
fn an_item_without_a_to_room_from_an_older_server_parses_as_an_empty_one() {
    let item = klatch::client::ReviewItem::from_wire(&serde_json::json!({
        "id": 4, "room": "vault", "seq": 2, "to_label": "public", "reason": "crossing", "text": "t"
    }));
    assert_eq!(
        (item.id, item.to_room.as_str(), item.to_label.as_str()),
        (4, "", "public")
    );
}
