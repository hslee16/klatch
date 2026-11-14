//! End-to-end tests: run the real binary against a throwaway KLATCH_HOME.
mod common;

use common::*;
use klatch_protocol::{ServerFrame, decode_server_line};
use std::io::{BufRead, BufReader};

#[test]
fn stream_prints_hello_the_history_and_caught_up_then_exits() {
    let h = Home::new();
    for i in 0..3 {
        h.ok("bob", &["post", "r", &format!("m{i}")]);
    }
    let f = h.stream("alice", &["r"]);
    assert!(matches!(&f[0], ServerFrame::Hello { member, .. } if member == "alice"));
    assert_eq!(seqs_of(&f), vec![1, 2, 3]);
    assert!(matches!(f.last().unwrap(), ServerFrame::CaughtUp { room, seq: 3, .. } if room == "r"));
}

#[test]
fn stream_from_seq_skips_what_the_consumer_already_has() {
    let h = Home::new();
    for i in 0..4 {
        h.ok("bob", &["post", "r", &format!("m{i}")]);
    }
    assert_eq!(
        seqs_of(&h.stream("alice", &["r", "--from-seq", "2"])),
        vec![3, 4]
    );
}

#[test]
fn stream_of_an_empty_room_is_hello_and_caught_up_at_zero() {
    let h = Home::new();
    let f = h.stream("alice", &["r"]);
    assert_eq!(f.len(), 2);
    assert!(matches!(&f[1], ServerFrame::CaughtUp { seq: 0, .. }));
}

#[test]
fn stream_has_no_window_so_a_reader_that_never_acknowledges_gets_everything() {
    let h = Home::new();
    // several posters, because one member is limited to a burst of 100 posts a minute per room
    for who in 0..5 {
        let posts: Vec<String> = (0..60)
            .map(|i| {
                request(
                    &format!("p{i}"),
                    "post",
                    &format!(r#"{{"room":"r","body":"m{who}-{i}"}}"#),
                )
            })
            .collect();
        let refs: Vec<&str> = posts.iter().map(String::as_str).collect();
        h.serve(&format!("bob{who}"), &refs);
    }
    assert_eq!(seqs_of(&h.stream("alice", &["r"])).len(), 300);
}

#[test]
fn stream_refuses_a_bad_room_before_printing_anything() {
    let h = Home::new();
    let o = h.run("alice", &["stream", "bad room"]);
    assert!(!o.status.success());
    assert!(o.stdout.is_empty(), "stdout must carry frames only");
    assert!(String::from_utf8_lossy(&o.stderr).contains("bad room name"));
}

#[test]
fn stream_still_requires_a_valid_member_name() {
    let h = Home::new();
    assert!(h.fail("bad name", &["stream", "r"]).contains("KLATCH_NAME"));
}

#[test]
fn stream_follow_pushes_a_live_message_and_can_be_stopped() {
    let h = Home::new();
    let mut child = h.spawn("alice", &["stream", "r", "--follow", "--poll-ms", "10"]);
    let mut lines = BufReader::new(child.stdout.take().unwrap());
    let mut next = || {
        let mut l = String::new();
        lines.read_line(&mut l).unwrap();
        decode_server_line(l.trim_end()).unwrap_or_else(|e| panic!("not a frame: {l}: {e:?}"))
    };
    assert!(matches!(next(), ServerFrame::Hello { .. }));
    assert!(matches!(next(), ServerFrame::CaughtUp { seq: 0, .. }));
    h.ok("bob", &["post", "r", "pushed"]);
    assert!(matches!(next(), ServerFrame::Message { seq: 1, ref body, .. } if body == "pushed"));
    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn the_cli_stream_and_a_serve_subscription_produce_identical_events() {
    // The conformance scenario: the same messages, read through `klatch stream` and through a
    // `subscribe` request on `klatch serve --stdio`, must give the same message and caught_up frames.
    let h = Home::new();
    h.ok("bob", &["post", "r", "first"]);
    h.ok("carol", &["post", "r", "second\nwith a newline"]);
    h.ok("bob", &["post", "r", "third"]);
    let via_stream = h.stream("alice", &["r", "--from-seq", "1"]);
    let via_serve = h.serve(
        "alice",
        &[&request("s", "subscribe", r#"{"room":"r","from_seq":1}"#)],
    );
    let events = |f: &[ServerFrame]| -> Vec<String> {
        f.iter()
            .filter(|x| {
                matches!(
                    x,
                    ServerFrame::Message { .. } | ServerFrame::CaughtUp { .. }
                )
            })
            .map(klatch_protocol::encode_line)
            .collect()
    };
    assert_eq!(events(&via_stream), events(&via_serve));
    assert_eq!(seqs_of(&via_stream), vec![2, 3]);
}

#[test]
fn messages_posted_from_the_cli_are_unattributed_on_the_stream() {
    let h = Home::new();
    h.ok("bob", &["post", "r", "from the env var"]);
    let f = h.stream("alice", &["r"]);
    let m = f
        .iter()
        .find(|x| matches!(x, ServerFrame::Message { .. }))
        .unwrap();
    assert!(
        matches!(m, ServerFrame::Message { attribution: klatch_protocol::Attribution::Unattributed, label, .. } if label == "public")
    );
}
