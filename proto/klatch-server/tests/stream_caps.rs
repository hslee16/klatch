//! What one credential can hold on `GET /v1/stream` (#1517, the same guard as the WebSocket route's, #112): a cap of streams
//! per member and a maximum stream age. The server of these tests runs with `KLATCH_TEST_SHORT_LIMITS` (a maximum age may be
//! as low as 1 s).

mod tls_common;

use std::io::{Read, Write};
use std::time::{Duration, Instant};
use tls_common::wire::*;
use tls_common::ws::*;
use tls_common::*;

const SLACK: Duration = Duration::from_secs(10);

fn stream_status(rig: &PlainRig, member: &str, token: &str) -> u16 {
    let bearer = format!("Bearer {token}");
    status_of(
        &rig.net,
        ("GET", "/v1/stream"),
        &[("X-Klatch-Member", member), ("Authorization", &bearer)],
        b"",
    )
}

fn open(rig: &PlainRig, who: usize) -> Sse {
    let name = ["alice", "bob"][who];
    Sse::open(&rig.net, name, &rig.tokens[who])
}

fn status_of_who(rig: &PlainRig, who: usize) -> u16 {
    stream_status(rig, ["alice", "bob"][who], &rig.tokens[who])
}

/// Try to open a stream for `who` until it is let in (a slot given back is seen by the server a moment after the client
/// lets go); the number of tries is bounded.
fn opens_soon(rig: &PlainRig, who: usize) -> bool {
    let until = Instant::now() + SLACK;
    while Instant::now() < until {
        if status_of_who(rig, who) == 200 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// A stream for `who`, retried while the server still counts a place the client has let go.
fn open_soon(rig: &PlainRig, who: usize) -> Sse {
    let name = ["alice", "bob"][who];
    let bearer = format!("Bearer {}", rig.tokens[who]);
    let until = Instant::now() + SLACK;
    loop {
        let c = rig.net.request(
            ("GET", "/v1/stream"),
            &[("X-Klatch-Member", name), ("Authorization", &bearer)],
            b"",
        );
        let mut r = std::io::BufReader::new(c);
        let (status, headers) = read_head(&mut r);
        if status == 200 {
            let session = headers
                .iter()
                .find(|(h, _)| h == "x-klatch-session")
                .map(|(_, v)| v.clone())
                .unwrap();
            return Sse {
                net: rig.net.clone(),
                member: name.into(),
                bearer,
                session,
                r,
                extra: Vec::new(),
                next: 0,
            };
        }
        assert!(Instant::now() < until, "refused for too long: {status}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn a_member_at_her_cap_cannot_open_another_stream_but_another_member_can() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&["--stream-max-per-member", "2"]);
    let (a1, a2) = (open(&rig, 0), open(&rig, 0));
    assert_eq!(status_of_who(&rig, 0), 503, "alice's third stream");
    // bob is not held out by alice's two
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
fn the_default_cap_is_four_and_the_refusal_is_the_one_a_full_server_gives_anyone() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&[]);
    let held: Vec<Sse> = (0..4).map(|_| open(&rig, 0)).collect();
    let refused = answer_or_stream_head(&rig.net, &stream_request("alice", &rig.tokens[0]));
    let full = plain_server(&["--max-connections", "1"]);
    let _holder = open(&full, 1);
    let when_full = answer_or_stream_head(&full.net, &stream_request("alice", &full.tokens[0]));
    assert!(refused.starts_with("HTTP/1.1 503"), "{refused:?}");
    assert_eq!(refused, when_full);
    drop(held);
}

/// What the server answers to `head`, to the end of the connection or for at most a few seconds: a refusal ends the
/// connection, but a stream wrongly let in never does (heartbeats keep it open), and reading to its end would hang instead
/// of letting the assertion fail.
fn answer_or_stream_head(net: &Net, head: &str) -> String {
    let mut c = net.connect();
    c.set_read_timeout(Duration::from_secs(3));
    c.write_all(head.as_bytes()).unwrap();
    c.flush().unwrap();
    let mut out = Vec::new();
    let mut buf = [0u8; 512];
    let until = Instant::now() + Duration::from_secs(3);
    while Instant::now() < until {
        match c.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn stream_request(member: &str, token: &str) -> String {
    format!(
        "GET /v1/stream HTTP/1.1\r\nHost: localhost\r\nX-Klatch-Member: {member}\r\nAuthorization: Bearer {token}\r\n\r\n"
    )
}

#[test]
fn the_caps_of_the_two_routes_are_counted_apart() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&["--stream-max-per-member", "1", "--ws-max-per-member", "1"]);
    let _s = open(&rig, 0);
    let mut w = rig.net.ws_open("alice", &rig.tokens[0]);
    w.set_timeout(Duration::from_secs(15));
    assert_eq!(w.recv().unwrap()["type"], "hello");
    assert_eq!(status_of_who(&rig, 0), 503);
}

#[test]
fn the_member_counted_is_the_one_the_credential_proves_and_a_refused_request_holds_no_slot() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&["--stream-max-per-member", "1"]);
    // wrong token for alice, bob's token under alice's name, alice's under bob's, and no credential at all: all 401, and
    // none of them uses up alice's or bob's place
    for _ in 0..3 {
        assert_eq!(stream_status(&rig, "alice", "not-a-token"), 401);
        assert_eq!(stream_status(&rig, "alice", &rig.tokens[1]), 401);
        assert_eq!(stream_status(&rig, "bob", &rig.tokens[0]), 401);
    }
    // an identity header that is not the credential's names nobody
    let forged = format!(
        "GET /v1/stream HTTP/1.1\r\nHost: localhost\r\nX-Klatch-Member: alice\r\nX-Forwarded-User: bob\r\nAuthorization: Bearer {}\r\n\r\n",
        rig.tokens[0]
    );
    let held = Sse::open(&rig.net, "alice", &rig.tokens[0]);
    assert!(
        answer_or_stream_head(&rig.net, &forged).starts_with("HTTP/1.1 503"),
        "alice's own cap is hit whatever a forwarded header says"
    );
    // bob still has his one
    let _b = open(&rig, 1);
    drop(held);
}

#[test]
fn a_stream_that_ends_any_way_gives_its_member_slot_back() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&["--stream-max-per-member", "1"]);
    for round in 0..3 {
        let s = open_soon(&rig, 0);
        match round {
            // the client says goodbye and the session ends
            0 => {
                let _ = s.post_frame(&serde_json::json!({"v":1,"type":"request","id":"b","method":"bye","params":{}}));
            }
            // the client leaves with nothing said
            1 => {}
            // a stream whose input is a frame the session cannot take is cut, or stays: either way a drop frees it
            _ => {
                let _ = s.post_frame(&serde_json::json!({"not":"a frame"}));
            }
        }
        drop(s);
    }
    assert!(opens_soon(&rig, 0), "slot not given back");
}

#[test]
fn a_stream_past_the_maximum_age_is_ended_and_the_member_reconnects() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&["--stream-max-age-secs", "3", "--stream-max-per-member", "1"]);
    let mut s = open(&rig, 0);
    let started = Instant::now();
    while s.recv().is_some() {
        assert!(
            started.elapsed() < SLACK,
            "still open after {:?}",
            started.elapsed()
        );
    }
    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "ended early: {:?}",
        started.elapsed()
    );
    // the slot came back with the end of the stream
    assert!(opens_soon(&rig, 0));
}

#[test]
fn the_stream_flags_refuse_zero_and_the_largest_number_at_start() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    for flag in ["--stream-max-per-member", "--stream-max-age-secs"] {
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
fn the_stream_age_has_a_floor_of_a_minute_and_a_ceiling_of_a_day_for_the_server_an_operator_runs() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    for bad in ["59", "86401"] {
        let dir = Dir::new();
        let args = vec![
            "--listen".into(),
            "127.0.0.1:0".into(),
            "--stream-max-age-secs".into(),
            bad.into(),
        ];
        let (code, log) = refused_start_strict(&dir, "http", &args);
        assert_eq!(code, Some(1), "{bad}: {log}");
        assert!(log.contains("60") && log.contains("86400"), "{bad}: {log}");
    }
}

#[test]
fn the_stream_cap_cannot_exceed_the_cap_of_connections_whatever_the_order_of_the_flags() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    for order in [
        ["--stream-max-per-member", "4", "--max-connections", "3"],
        ["--max-connections", "3", "--stream-max-per-member", "4"],
    ] {
        let dir = Dir::new();
        let mut args: Vec<String> = vec!["--listen".into(), "127.0.0.1:0".into()];
        args.extend(order.iter().map(|s| s.to_string()));
        let (code, log) = refused_start(&dir, "http", &args);
        assert_eq!(code, Some(1), "{order:?}: {log}");
        assert!(
            log.contains("--stream-max-per-member") && log.contains("--max-connections"),
            "{log}"
        );
    }
}
