//! What one credential can hold on the WebSocket route (#112, review of #1513): a cap of connections per member, a maximum
//! connection age, the close of a socket whose credential was revoked, and the refusal of an HTTP/1.0 upgrade. The server of
//! these tests runs with `KLATCH_TEST_SHORT_LIMITS` (idle 3 s, ping 1 s, close wait 1 s; a maximum age may be as low as 1 s).

mod tls_common;

use serde_json::json;
use std::time::{Duration, Instant};
use tls_common::wire::*;
use tls_common::ws::*;
use tls_common::*;

const SLACK: Duration = Duration::from_secs(10);

fn open(rig: &PlainRig, who: usize) -> Ws {
    let name = ["alice", "bob"][who];
    let mut c = rig.net.ws_open(name, &rig.tokens[who]);
    c.set_timeout(Duration::from_secs(15));
    assert_eq!(c.recv().unwrap()["type"], "hello");
    c
}

fn status_of_upgrade(rig: &PlainRig, who: usize) -> u16 {
    let name = ["alice", "bob"][who];
    let bearer = format!("Bearer {}", rig.tokens[who]);
    rig.net
        .ws_upgrade(&[("X-Klatch-Member", name), ("Authorization", &bearer)])
        .0
}

/// Try to open a connection for `who` until it is let in (a slot given back is seen by the server a moment after the
/// client lets go); the number of tries is bounded.
fn opens_soon(rig: &PlainRig, who: usize) -> bool {
    let until = Instant::now() + SLACK;
    while Instant::now() < until {
        if status_of_upgrade(rig, who) == 101 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

// ---------------------------------------------------------------------------------------------------------------
// a cap per member

#[test]
fn a_member_at_her_cap_cannot_open_another_connection_but_another_member_can() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&["--ws-max-per-member", "2"]);
    let (a1, a2) = (open(&rig, 0), open(&rig, 0));
    assert_eq!(status_of_upgrade(&rig, 0), 503, "alice's third connection");
    // bob is not held out by alice's two, and alice's cap is her own
    let _b = open(&rig, 1);
    // a slot given back is usable again
    drop(a1);
    assert!(
        opens_soon(&rig, 0),
        "alice could not reconnect after closing one"
    );
    drop(a2);
}

#[test]
fn the_default_cap_is_four_and_a_refusal_at_it_is_the_refusal_of_a_full_server_and_names_no_one() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&[]);
    let held: Vec<Ws> = (0..4).map(|_| open(&rig, 0)).collect();
    let alice_refused = raw_answer(&rig.net, &upgrade_for("alice", &rig.tokens[0]));
    // the same bytes a server that is full gives anyone (here: a server of one connection, held by someone else)
    let full = plain_server(&["--max-connections", "1"]);
    let _holder = open(&full, 1);
    let when_full = raw_answer(&full.net, &upgrade_for("alice", &full.tokens[0]));
    assert!(
        alice_refused.starts_with("HTTP/1.1 503"),
        "{alice_refused:?}"
    );
    assert_eq!(alice_refused, when_full);
    drop(held);
}

#[test]
fn a_connection_that_ends_any_way_gives_its_member_slot_back() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&["--ws-max-per-member", "1"]);
    for round in 0..3 {
        let mut c = open(&rig, 0);
        match round {
            0 => {
                c.close(1000);
                c.until_end();
            }
            1 => c.send(true, BINARY, b"x"),
            _ => {}
        }
        drop(c);
        assert!(opens_soon(&rig, 0), "slot not given back after way {round}");
        // that connection is dropped at once, and the next round starts from a clean cap
        let mut again = open(&rig, 0);
        again.close(1000);
        again.until_end();
        assert!(opens_soon(&rig, 0));
    }
}

// ---------------------------------------------------------------------------------------------------------------
// the flags

#[test]
fn the_two_flags_refuse_zero_and_the_largest_number_at_start() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    for flag in ["--ws-max-per-member", "--ws-max-age-secs"] {
        for bad in ["0", "18446744073709551615", "x", "-1", ""] {
            let dir = Dir::new();
            let args = vec![
                "--listen".into(),
                "127.0.0.1:0".into(),
                flag.to_string(),
                bad.into(),
            ];
            let (code, log) = refused_start(&dir, "http", &args);
            assert_eq!(code, Some(1), "{flag} {bad:?}: {log}");
            assert!(log.contains(flag), "{flag} {bad:?}: {log}");
        }
    }
}

#[test]
fn the_age_has_a_floor_of_a_minute_and_a_ceiling_of_a_day_for_the_server_an_operator_runs() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    for bad in ["59", "86401"] {
        let dir = Dir::new();
        let args = vec![
            "--listen".into(),
            "127.0.0.1:0".into(),
            "--ws-max-age-secs".into(),
            bad.into(),
        ];
        let (code, log) = refused_start_strict(&dir, "http", &args);
        assert_eq!(code, Some(1), "{bad}: {log}");
        assert!(log.contains("60") && log.contains("86400"), "{bad}: {log}");
    }
}

#[test]
fn the_cap_per_member_cannot_exceed_the_cap_of_connections_whatever_the_order_of_the_flags() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    for order in [
        ["--ws-max-per-member", "4", "--max-connections", "3"],
        ["--max-connections", "3", "--ws-max-per-member", "4"],
    ] {
        let dir = Dir::new();
        let mut args: Vec<String> = vec!["--listen".into(), "127.0.0.1:0".into()];
        args.extend(order.iter().map(|s| s.to_string()));
        let (code, log) = refused_start(&dir, "http", &args);
        assert_eq!(code, Some(1), "{order:?}: {log}");
        assert!(
            log.contains("--ws-max-per-member") && log.contains("--max-connections"),
            "{log}"
        );
    }
}

// ---------------------------------------------------------------------------------------------------------------
// the maximum age

#[test]
fn a_connection_past_the_maximum_age_is_closed_going_away_even_if_it_answers_every_ping() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&["--ws-max-age-secs", "3"]);
    let mut c = open(&rig, 0);
    c.set_timeout(Duration::from_millis(500));
    let started = Instant::now();
    let close = loop {
        match c.got() {
            Got::Frame(Frame::Ping(p)) => c.send(true, PONG, &p),
            Got::Frame(Frame::Close(code, reason)) => break (code, reason),
            Got::Frame(Frame::Text(_)) | Got::Quiet => {}
            other => panic!("{other:?} before the age was up"),
        }
        assert!(
            started.elapsed() < SLACK,
            "still open after {:?}",
            started.elapsed()
        );
    };
    assert_eq!(close.0, Some(1001), "{close:?}");
    assert!(close.1.contains("age"), "{close:?}");
    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "closed early: {:?}",
        started.elapsed()
    );
    // and a reconnect resumes: the same member opens again at once
    c.close(1001);
    assert!(opens_soon(&rig, 0));
}

// ---------------------------------------------------------------------------------------------------------------
// a revoked credential

fn revoked_rig() -> (PlainRig, Ws, Ws) {
    let rig = plain_server(&[]);
    let mut a = open(&rig, 0);
    let mut b = open(&rig, 1);
    for c in [&mut a, &mut b] {
        assert_eq!(c.request("join", json!({"room": "rev"}))["ok"], true);
        assert_eq!(
            c.request("subscribe", json!({"room": "rev", "from_seq": 0}))["ok"],
            true
        );
    }
    assert_eq!(b.post("rev", "before"), 1);
    revoke_credential(&rig.dir, "alice");
    (rig, a, b)
}

/// The frames until the connection ends (a close or a cut), pings skipped.
fn frames_to_the_end(c: &mut Ws) -> Vec<Frame> {
    c.until_end()
        .into_iter()
        .filter(|f| !matches!(f, Frame::Ping(_)))
        .collect()
}

#[test]
fn a_request_on_a_socket_whose_credential_was_revoked_is_refused_and_the_socket_is_closed_1008() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let (rig, mut a, _b) = revoked_rig();
    a.text(&json!({"v":1,"type":"request","id":"after","method":"history","params":{"room":"rev","last":1}}).to_string());
    let frames = frames_to_the_end(&mut a);
    let texts: Vec<serde_json::Value> = frames
        .iter()
        .filter_map(|f| match f {
            Frame::Text(t) => serde_json::from_str(t).ok(),
            _ => None,
        })
        .collect();
    assert!(
        texts
            .iter()
            .any(|f| f["type"] == "refusal" && f["rule"] == "credential"),
        "{frames:?}"
    );
    assert!(
        matches!(frames.last(), Some(Frame::Close(Some(1008), _))),
        "{frames:?}"
    );
    // a new upgrade is the one 401
    assert_eq!(status_of_upgrade(&rig, 0), 401);
}

#[test]
fn a_message_posted_after_the_revoke_does_not_reach_the_revoked_member() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let (_rig, mut a, mut b) = revoked_rig();
    assert_eq!(b.post("rev", "after the revoke"), 2);
    let frames = frames_to_the_end(&mut a);
    for f in &frames {
        if let Frame::Text(t) = f {
            assert!(
                !t.contains("after the revoke"),
                "the revoked member was sent {t}"
            );
        }
    }
    assert!(
        matches!(frames.last(), Some(Frame::Close(Some(1008), _))),
        "{frames:?}"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// HTTP/1.0

#[test]
fn an_http_1_0_upgrade_is_refused_after_the_credential() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&[]);
    let good = upgrade_for("alice", &rig.tokens[0]);
    assert_eq!(status_of_upgrade(&rig, 0), 101);
    let old = good.replace("HTTP/1.1", "HTTP/1.0");
    let answer = raw_answer(&rig.net, &old);
    assert!(answer.starts_with("HTTP/1.1 400"), "{answer:?}");
    assert!(!answer.contains("Sec-WebSocket-Accept"), "{answer:?}");
    // without the credential it is the 401 first
    let stranger = upgrade_request(&[]).replace("HTTP/1.1", "HTTP/1.0");
    assert!(raw_answer(&rig.net, &stranger).starts_with("HTTP/1.1 401"));
}
