//! The wire behind the mutual TLS listener (#108) is the loopback HTTP wire (#1059): the same client code runs one
//! scenario against `klatch-server http` (plain, loopback) and `klatch-server https` (mutual TLS) and gets the same frames,
//! the same sequence numbers and the same errors; a stream resumes by `seq` over TLS; a wrong credential is the one `401`
//! behind a valid certificate; SIGTERM closes open TLS streams. Certificates are throwaway (`tls_common`).

mod tls_common;

use serde_json::{Value, json};
use std::time::Duration;
use tls_common::scenario::*;
use tls_common::wire::*;
use tls_common::*;

// ---------------------------------------------------------------------------------------------------------------

#[test]
fn behind_a_valid_certificate_every_route_still_needs_a_live_credential_for_the_member_named() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let t = tls_server();
    let ok = format!("Bearer {}", t.tokens[0]);
    let bob_with_alices = [("X-Klatch-Member", "bob"), ("Authorization", ok.as_str())];
    for (target, body) in [
        (("GET", "/v1/stream"), &b""[..]),
        (("POST", "/v1/frames"), &b"{}\n"[..]),
        (("POST", "/mcp"), &b"{}"[..]),
    ] {
        assert_eq!(
            status_of(&t.net, target, &[], body),
            401,
            "{target:?} without a credential"
        );
        assert_eq!(
            status_of(&t.net, target, &bob_with_alices, body),
            401,
            "{target:?} with another member's token"
        );
        // headers a proxy would add give no identity, certificate or not
        let forged = [
            ("X-Forwarded-User", "alice"),
            ("X-Client-Cert", "CN=alice"),
            ("X-Remote-User", "alice"),
        ];
        assert_eq!(
            status_of(&t.net, target, &forged, body),
            401,
            "{target:?} with forwarded identity"
        );
    }
    // and the right credential gets in over the same certificate
    let nowhere = "0123456789abcdef0123456789abcdef";
    let good = [
        ("X-Klatch-Member", "alice"),
        ("Authorization", ok.as_str()),
        ("X-Klatch-Session", nowhere),
    ];
    assert_eq!(
        status_of(&t.net, ("POST", "/v1/frames"), &good, b"{}\n"),
        404,
        "a valid credential, no such session"
    );
    drop(t.server);
}

#[test]
fn a_stream_resumes_by_sequence_number_over_tls() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let t = tls_server();
    let mut a = Sse::open(&t.net, "alice", &t.tokens[0]);
    assert_eq!(a.recv().unwrap()["attribution"], "verified");
    assert_eq!(a.request("join", json!({"room": "res"}))["ok"], true);
    let seqs: Vec<i64> = (0..4).map(|i| a.post("res", &format!("m{i}"))).collect();
    assert_eq!(seqs, [1, 2, 3, 4]);
    drop(a);
    // a new stream asks for everything after the second message and gets the same messages with the same numbers
    let mut b = Sse::open(&t.net, "alice", &t.tokens[0]);
    b.recv().unwrap();
    assert_eq!(
        b.request("subscribe", json!({"room": "res", "from_seq": 2}))["ok"],
        true
    );
    let mut got = Vec::new();
    while got.len() < 2 {
        let f = b.recv().expect("the resumed messages");
        if f["type"] == "message" {
            got.push((
                f["seq"].as_i64().unwrap(),
                f["body"].as_str().unwrap().to_string(),
            ));
        }
    }
    assert_eq!(got, [(3, "m2".to_string()), (4, "m3".to_string())]);
}

#[test]
fn sigterm_closes_open_tls_streams_and_stops_the_server() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let mut t = tls_server();
    let mut a = Sse::open(&t.net, "alice", &t.tokens[0]);
    a.recv().unwrap();
    unsafe {
        libc::kill(t.server.child.id() as libc::pid_t, libc::SIGTERM);
    }
    let until = std::time::Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(s) = t.server.child.try_wait().unwrap() {
            break s;
        }
        assert!(std::time::Instant::now() < until, "the server did not stop");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "{}", t.dir.log());
    assert!(a.recv().is_none(), "the stream ended");
    assert!(t.dir.log().contains("https stopped"));
}

// ---------------------------------------------------------------------------------------------------------------
// conformance: one scenario, one client, two listeners

#[test]
fn the_same_scenario_over_loopback_http_and_over_mutual_tls_gives_the_same_frames() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let plain_dir = Dir::new();
    let plain_tokens = [
        issue_credential(&plain_dir, "alice"),
        issue_credential(&plain_dir, "bob"),
    ];
    let plain_server = Server::started(&plain_dir, "http", &[]);
    let plain = Net {
        addr: plain_server.addr,
        tls: None,
    };
    let t = tls_server();
    for (i, name) in ["alice", "bob"].into_iter().enumerate() {
        let over_http = run_scenario(&plain, name, &plain_tokens[i]);
        let over_tls = run_scenario(&t.net, name, &t.tokens[i]);
        let (ra, sa) = split(&over_http);
        let (rb, sb) = split(&over_tls);
        assert!(ra.len() >= 10 && !sa.is_empty(), "{name}: {over_http:?}");
        assert_eq!(ra, rb, "{name}: responses");
        assert_eq!(sa, sb, "{name}: stream frames");
        let seqs: Vec<i64> = rb
            .iter()
            .filter(|f| f["id"].as_str().is_some_and(|i| i.starts_with('p')))
            .map(|f| f["result"]["seq"].as_i64().unwrap())
            .collect();
        assert_eq!(seqs.len(), 4, "{name}: {seqs:?}");
    }
}
