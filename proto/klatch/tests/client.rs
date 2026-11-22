//! The conformance scenario for a klatch client (#119), in Rust: the same steps as the Python and TypeScript clients in
//! `clients/`, run against the real binary through `klatch::client`.
mod common;

use common::*;
use klatch::client::{ClaimRequest, Client, PostRequest};
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
