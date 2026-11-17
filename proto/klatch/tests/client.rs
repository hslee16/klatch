//! The conformance scenario for a klatch client (#119), in Rust: the same steps as the Python and TypeScript clients in
//! `clients/`, run against the real binary through `klatch::client`.
mod common;

use common::*;
use klatch::client::{ClaimRequest, Client, Error, PostRequest};
use klatch::protocol::{PROTOCOL_VERSION, decode_server_line};
use std::path::PathBuf;
use std::time::Duration;

fn connect(h: &Home, name: &str) -> Client {
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

fn api(e: Error) -> klatch::client::ApiError {
    match e {
        Error::Api(a) => a,
        other => panic!("not an api error: {other}"),
    }
}

#[test]
fn the_protocol_version_is_one_and_hello_says_who_you_are() {
    assert_eq!(PROTOCOL_VERSION, 1);
    let h = Home::new();
    let a = connect(&h, "alice");
    assert_eq!(
        (
            a.member.as_str(),
            a.attribution.as_str(),
            a.clearance.as_str()
        ),
        ("alice", "unattributed", "public")
    );
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
fn a_post_is_read_by_another_member_and_a_handoff_keeps_its_kind_and_payload() {
    let h = Home::new();
    let (mut a, mut b) = (connect(&h, "alice"), connect(&h, "bob"));
    a.post(PostRequest::new("r", "just words")).unwrap();
    a.post(PostRequest::new("r", "see the order").handoff("work-order", Some("{\"id\":7}")))
        .unwrap();
    let ms = b.read("r", true).unwrap();
    assert_eq!(
        ms.iter().map(|m| m.body.as_str()).collect::<Vec<_>>(),
        vec!["just words", "see the order"]
    );
    assert_eq!(
        (ms[0].kind.as_deref(), ms[0].payload.as_deref()),
        (None, None)
    );
    assert_eq!(
        (ms[1].kind.as_deref(), ms[1].payload.as_deref()),
        (Some("work-order"), Some("{\"id\":7}"))
    );
    assert_eq!(ms[0].attribution, "unattributed");
    assert_eq!(b.read("r", false).unwrap().len(), 2);
    assert!(b.read("r", false).unwrap().is_empty());
}

#[test]
fn subscribe_replays_catches_up_pushes_and_resumes_after_a_reconnect() {
    let h = Home::new();
    let (mut a, mut b) = (connect(&h, "alice"), connect(&h, "bob"));
    for i in 0..3 {
        a.post(PostRequest::new("r", &format!("m{i}"))).unwrap();
    }
    assert_eq!(b.subscribe("r", 0).unwrap(), 3);
    let seen = b.collect(true, Duration::from_secs(10)).unwrap();
    assert_eq!(
        seen.messages.iter().map(|m| m.seq).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(seen.caught_up, Some(3));
    a.post(PostRequest::new("r", "live")).unwrap();
    let live = b.next_message(Duration::from_secs(5)).unwrap().unwrap();
    assert_eq!((live.seq, live.body.as_str()), (4, "live"));
    b.ack("r", 4).unwrap();
    drop(b);
    a.post(PostRequest::new("r", "while away")).unwrap();
    let mut b2 = connect(&h, "bob");
    b2.subscribe("r", 4).unwrap();
    let resumed = b2.collect(true, Duration::from_secs(10)).unwrap();
    assert_eq!(
        resumed.messages.iter().map(|m| m.seq).collect::<Vec<_>>(),
        vec![5]
    );
}

#[test]
fn a_second_claim_conflicts_names_the_holder_and_a_release_frees_the_path() {
    let h = Home::new();
    let (mut a, mut b) = (connect(&h, "alice"), connect(&h, "bob"));
    a.claim(
        ClaimRequest::new("r", "src/a.rs")
            .ttl(60)
            .reason("refactor"),
    )
    .unwrap();
    let e = api(b.claim(ClaimRequest::new("r", "src/a.rs")).unwrap_err());
    assert_eq!(e.code, "conflict");
    assert!(e.message.contains("alice"), "{}", e.message);
    assert_eq!(b.claim_owners("r").unwrap(), vec!["alice".to_string()]);
    a.release("r", "src/a.rs", Some("done")).unwrap();
    b.claim(ClaimRequest::new("r", "src/a.rs")).unwrap();
    assert_eq!(
        api(a.release("r", "never/claimed", None).unwrap_err()).code,
        "not_found"
    );
}

#[test]
fn a_refusal_carries_its_rule_and_never_repeats_the_text() {
    let h = Home::new();
    let mut a = connect(&h, "alice");
    let secret = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    let e = api(a
        .post(PostRequest::new("r", &format!("token {secret}")))
        .unwrap_err());
    assert_eq!(e.code, "refused");
    assert!(
        e.refusals.iter().any(|(rule, _)| rule == "credentials"),
        "{:?}",
        e.refusals
    );
    assert!(!format!("{:?}{}", e.refusals, e.message).contains(secret));
}

#[test]
fn bad_requests_get_a_failed_response_and_the_session_carries_on() {
    let h = Home::new();
    let mut a = connect(&h, "alice");
    assert_eq!(
        api(a
            .request("no_such_method", serde_json::json!({}))
            .unwrap_err())
        .code,
        "unknown_method"
    );
    assert_eq!(
        api(a
            .request("post", serde_json::json!({"room": "r"}))
            .unwrap_err())
        .code,
        "invalid_params"
    );
    assert_eq!(
        api(a.post(PostRequest::new("bad room name", "x")).unwrap_err()).code,
        "invalid"
    );
    assert!(
        a.post(PostRequest::new("r", "p").handoff("", Some("needs a kind")))
            .is_err()
    );
    assert_eq!(a.post(PostRequest::new("r", "still working")).unwrap().0, 1);
}

#[test]
fn a_member_without_the_admin_role_is_refused_every_act_and_may_prune() {
    let h = Home::new();
    let mut a = connect(&h, "alice");
    for (act, params) in [
        ("run-create", serde_json::json!({"run": "r1"})),
        ("run-end", serde_json::json!({"run": "r1"})),
        ("anchor-done", serde_json::json!({"anchor": "gh:pr:o/r/1"})),
        ("close", serde_json::json!({"room": "r"})),
    ] {
        assert_eq!(
            api(a.role_act(act, params).unwrap_err()).code,
            "refused",
            "{act}"
        );
    }
    assert_eq!(
        api(a
            .role_act("reap-anchors", serde_json::json!({}))
            .unwrap_err())
        .code,
        "unknown_act"
    );
    assert_eq!(
        api(a.role_act("run-create", serde_json::json!({})).unwrap_err()).code,
        "invalid_params"
    );
    assert_eq!(a.prune().unwrap(), 0);
    a.post(PostRequest::new("r", "still here")).unwrap();
    assert_eq!(a.prune().unwrap(), 0);
}

#[test]
fn a_direct_message_reaches_one_member_in_a_room_nobody_else_may_use() {
    let h = Home::new();
    let (mut a, mut b, mut c) = (
        connect(&h, "alice"),
        connect(&h, "bob"),
        connect(&h, "carol"),
    );
    let (room, seq, id) = a.dm("bob", "hello bob", None).unwrap();
    assert_eq!((room.as_str(), seq, id.len()), ("dm.alice.bob", 1, 26));
    let got = b.read(&room, false).unwrap();
    assert_eq!(
        got.iter()
            .map(|m| (m.sender.as_str(), m.body.as_str()))
            .collect::<Vec<_>>(),
        vec![("alice", "hello bob")]
    );
    assert_eq!(b.dm("alice", "hi back", None).unwrap().1, 2);
    assert_eq!(api(c.read(&room, false).unwrap_err()).code, "refused");
    assert_eq!(
        api(a.dm("alice", "to myself", None).unwrap_err()).code,
        "invalid"
    );
    assert_eq!(
        api(a
            .dm("bob", "above the ceiling", Some("secret"))
            .unwrap_err())
        .code,
        "refused"
    );
}

#[test]
fn a_member_mutes_and_unmutes_a_room_and_read_still_shows_everything() {
    let h = Home::new();
    let (mut a, mut b) = (connect(&h, "alice"), connect(&h, "bob"));
    b.post(PostRequest::new("r", "chatter")).unwrap();
    a.mute("r", true).unwrap();
    assert_eq!(a.read("r", true).unwrap().len(), 1);
    a.mute("r", false).unwrap();
    a.mute("r", true).unwrap();
    assert_eq!(api(a.mute("bad room", true).unwrap_err()).code, "invalid");
}

#[test]
fn closing_the_input_ends_the_session_cleanly() {
    let h = Home::new();
    let mut a = connect(&h, "alice");
    a.post(PostRequest::new("r", "x")).unwrap();
    assert_eq!(a.close().unwrap(), 0);
}

#[test]
fn history_looks_back_without_moving_the_cursor_and_pages_with_since() {
    let h = Home::new();
    let (mut a, mut b) = (connect(&h, "alice"), connect(&h, "bob"));
    for i in 1..=5 {
        a.post(PostRequest::new("r", &format!("m{i}"))).unwrap();
    }
    let bodies = |ms: &[klatch::client::Message]| -> Vec<String> {
        ms.iter().map(|m| m.body.clone()).collect()
    };
    let all = b.history("r", None, None).unwrap();
    assert_eq!(bodies(&all.messages), vec!["m1", "m2", "m3", "m4", "m5"]);
    assert_eq!(all.more, 0);
    let last2 = b.history("r", Some(2), None).unwrap();
    assert_eq!(bodies(&last2.messages), vec!["m4", "m5"]);
    let since = all.messages[1].seq;
    let page = b.history("r", Some(2), Some(since)).unwrap();
    assert_eq!(bodies(&page.messages), vec!["m4", "m5"]);
    let next = b.history("r", None, Some(since)).unwrap();
    assert_eq!(bodies(&next.messages), vec!["m3", "m4", "m5"]);
    let own = a.history("r", Some(1), None).unwrap();
    assert_eq!(bodies(&own.messages), vec!["m5"]);
    assert_eq!(b.read("r", false).unwrap().len(), 5);
    assert_eq!(
        api(b.history("r", Some(0), None).unwrap_err()).code,
        "invalid"
    );
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
