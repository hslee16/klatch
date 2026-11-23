//! The frame layer of the WebSocket route (#112): what a client may send and how long the server waits. Every wait is
//! bounded, and a test that finds the server not bounded fails by assertion (a read timeout on the test's socket), never by
//! hanging. The server of these tests runs with `KLATCH_TEST_SHORT_LIMITS` (a debug build only), which makes the idle time
//! 3 seconds, the ping interval 1 second and the close wait 1 second; the shipped values are 60, 20 and 2.

mod tls_common;

use serde_json::json;
use std::time::{Duration, Instant};
use tls_common::wire::*;
use tls_common::ws::*;
use tls_common::*;

/// The limits the test server runs with (see the module text): the idle time and the close wait.
const IDLE: Duration = Duration::from_secs(3);
const CLOSE_WAIT: Duration = Duration::from_secs(1);
/// Slack for a slow machine; a server that is not bounded is far outside it.
const SLACK: Duration = Duration::from_secs(8);
const MIB: usize = 1024 * 1024;

fn open(rig: &PlainRig) -> Ws {
    let mut c = rig.net.ws_open("alice", &rig.tokens[0]);
    c.set_timeout(Duration::from_secs(15));
    assert_eq!(c.recv().unwrap()["type"], "hello");
    c
}

/// The frames until the end, which must end the connection within the slack, and the last one must be a close with `code`.
fn ends_with_close(c: &mut Ws, code: u16) {
    let started = Instant::now();
    let frames = c.until_end();
    assert!(
        started.elapsed() < SLACK,
        "the close took {:?}",
        started.elapsed()
    );
    match frames.last() {
        Some(Frame::Close(Some(got), _)) => assert_eq!(*got, code, "{frames:?}"),
        other => panic!("expected a close with {code}, got {other:?} in {frames:?}"),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// what a client may send

#[test]
fn a_client_frame_that_is_not_masked_is_a_protocol_error() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&[]);
    let mut c = open(&rig);
    c.send_raw(&encode(
        true,
        TEXT,
        br#"{"v":1,"type":"request","id":"x","method":"who","params":{}}"#,
        false,
    ))
    .unwrap();
    ends_with_close(&mut c, 1002);
}

#[test]
fn a_frame_that_declares_more_than_a_mebibyte_is_refused_before_its_payload_is_read() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&[]);
    let mut c = open(&rig);
    // the header alone, for 2 MiB, and not a byte of payload: the size is judged at the header
    c.send_raw(&header_only(TEXT, 2 * MIB as u64)).unwrap();
    ends_with_close(&mut c, 1009);
    // and a size no memory could hold is the same refusal, not an allocation
    let mut c = open(&rig);
    c.send_raw(&header_only(TEXT, u64::MAX >> 1)).unwrap();
    ends_with_close(&mut c, 1009);
}

#[test]
fn a_message_of_fragments_that_adds_up_to_more_than_a_mebibyte_is_refused() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&[]);
    let mut c = open(&rig);
    let chunk = vec![b'a'; MIB / 4];
    c.send(false, TEXT, &chunk);
    for _ in 0..4 {
        // each frame is small and legal; the message they make is not, and it is never held whole past the limit
        if c.send_raw(&encode(false, CONT, &chunk, true)).is_err() {
            break;
        }
    }
    ends_with_close(&mut c, 1009);
}

#[test]
fn a_message_in_fragments_is_one_frame_and_a_ping_may_come_between_its_parts() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&[]);
    let mut c = open(&rig);
    let line = json!({"v":1,"type":"request","id":"frag","method":"join","params":{"room":"frag"}})
        .to_string();
    let (a, rest) = line.split_at(10);
    let (b, d) = rest.split_at(15);
    c.send(false, TEXT, a.as_bytes());
    c.send(true, PING, b"between");
    c.send(false, CONT, b.as_bytes());
    c.send(true, CONT, d.as_bytes());
    let mut saw_pong = false;
    loop {
        match c.got() {
            Got::Frame(Frame::Pong(p)) => {
                assert_eq!(p, b"between");
                saw_pong = true;
            }
            Got::Frame(Frame::Text(t)) => {
                let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                if v["id"] == "frag" {
                    assert_eq!(v["ok"], true, "{v}");
                    break;
                }
            }
            other => panic!("{other:?}"),
        }
    }
    assert!(saw_pong, "the ping between the fragments was not answered");
}

#[test]
fn a_binary_frame_is_refused_and_so_is_text_that_is_not_utf_8() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&[]);
    let mut c = open(&rig);
    c.send(true, BINARY, b"{}");
    ends_with_close(&mut c, 1003);
    let mut c = open(&rig);
    c.send(true, TEXT, &[0xff, 0xfe, 0xfd]);
    ends_with_close(&mut c, 1007);
}

#[test]
fn a_control_frame_over_125_bytes_or_in_pieces_is_a_protocol_error() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&[]);
    let mut c = open(&rig);
    c.send(true, PING, &[b'p'; 126]);
    ends_with_close(&mut c, 1002);
    let mut c = open(&rig);
    c.send(false, PING, b"half");
    ends_with_close(&mut c, 1002);
    // a reserved bit with no extension negotiated, and an opcode that does not exist
    let mut c = open(&rig);
    let mut bad = encode(true, TEXT, b"{}", true);
    bad[0] |= 0x40;
    c.send_raw(&bad).unwrap();
    ends_with_close(&mut c, 1002);
    let mut c = open(&rig);
    c.send(true, 0x3, b"x");
    ends_with_close(&mut c, 1002);
}

#[test]
fn a_ping_is_answered_with_its_payload_and_a_flood_of_them_is_answered_without_growing() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&[]);
    let mut c = open(&rig);
    c.send(true, PING, b"one");
    assert_eq!(c.got(), Got::Frame(Frame::Pong(b"one".to_vec())));
    // a thousand pings at once: the server keeps the latest reply, not a thousand; the session still works
    let flood: Vec<u8> = (0..1000)
        .flat_map(|i| encode(true, PING, format!("p{i}").as_bytes(), true))
        .collect();
    c.send_raw(&flood).unwrap();
    assert_eq!(c.post_after_flood(), 1);
}

impl Ws {
    /// Join a room and post once, skipping the pongs that are still arriving.
    fn post_after_flood(&mut self) -> i64 {
        assert_eq!(
            self.request_skipping_pongs("join", json!({"room": "flood"}))["ok"],
            true
        );
        self.request_skipping_pongs("post", json!({"room": "flood", "body": "x"}))["result"]["seq"]
            .as_i64()
            .unwrap()
    }

    fn request_skipping_pongs(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> serde_json::Value {
        // `request` panics on a frame that is not text, so the pongs are read away first
        self.next += 1;
        let id = format!("f{}", self.next);
        self.text(
            &json!({"v":1,"type":"request","id":id,"method":method,"params":params}).to_string(),
        );
        loop {
            match self.got() {
                Got::Frame(Frame::Pong(_)) => {}
                Got::Frame(Frame::Text(t)) => {
                    let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                    if v["type"] == "response" && v["id"] == id.as_str() {
                        return v;
                    }
                }
                other => panic!("{other:?}"),
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// how long the server waits

#[test]
fn a_client_that_says_nothing_and_answers_no_ping_is_closed_when_the_idle_time_is_up() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&[]);
    let mut c = open(&rig);
    let started = Instant::now();
    let mut pings = 0;
    // read what the server sends (pings, then a close) and answer nothing
    let frames = loop {
        match c.got() {
            Got::Frame(Frame::Ping(_)) => pings += 1,
            Got::Frame(f @ Frame::Close(..)) => break f,
            other => panic!("{other:?}"),
        }
    };
    assert!(pings >= 1, "the server never pinged");
    assert!(matches!(frames, Frame::Close(Some(1001), _)), "{frames:?}");
    let took = started.elapsed();
    assert!(
        took >= IDLE - Duration::from_millis(500) && took < IDLE + SLACK,
        "{took:?}"
    );
    // the client never answers the close: the server cuts the connection after its own bound, not at the client's pleasure
    let t = Instant::now();
    assert_eq!(c.got(), Got::Eof);
    assert!(t.elapsed() < CLOSE_WAIT + SLACK, "{:?}", t.elapsed());
}

#[test]
fn a_client_that_answers_pings_is_not_idle_however_quiet_it_is() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&[]);
    let mut c = open(&rig);
    c.set_timeout(Duration::from_millis(500));
    let until = Instant::now() + 3 * IDLE;
    let mut pings = 0;
    while Instant::now() < until {
        match c.got() {
            Got::Frame(Frame::Ping(p)) => {
                pings += 1;
                c.send(true, PONG, &p);
            }
            Got::Frame(Frame::Text(_)) | Got::Quiet => {}
            other => panic!("closed while answering pings: {other:?}"),
        }
    }
    assert!(pings >= 3, "{pings} pings in {:?}", 3 * IDLE);
    c.set_timeout(Duration::from_secs(15));
    assert_eq!(c.post("alive", "x"), 1);
}

#[test]
fn a_close_from_the_client_is_answered_and_the_connection_ends() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&[]);
    let mut c = open(&rig);
    c.close(1000);
    ends_with_close(&mut c, 1000);
}

#[test]
fn a_client_that_sends_half_a_frame_and_stops_is_closed_when_the_idle_time_is_up() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&[]);
    let mut c = open(&rig);
    let whole = encode(
        true,
        TEXT,
        br#"{"v":1,"type":"request","id":"x","method":"who","params":{}}"#,
        true,
    );
    c.send_raw(&whole[..whole.len() / 2]).unwrap();
    // the half frame is held (a few bytes), never grown, and the connection is closed as an idle one is
    let started = Instant::now();
    let frames = c.until_end();
    assert!(
        started.elapsed() < IDLE + CLOSE_WAIT + SLACK,
        "{:?}",
        started.elapsed()
    );
    assert!(
        matches!(frames.last(), Some(Frame::Close(Some(1001), _))),
        "{frames:?}"
    );
}

#[test]
fn a_client_that_asks_for_a_lot_and_never_reads_is_cut_when_the_stall_time_is_up() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&[]);
    let mut c = open(&rig);
    assert_eq!(c.request("join", json!({"room": "big"}))["ok"], true);
    // 40 messages of 20 KiB, then the same history over and over without reading: far more than the socket buffers hold
    for i in 0..40 {
        let body = format!("{i}{}", "x".repeat(20_000));
        assert_eq!(
            c.request("post", json!({"room": "big", "body": body}))["ok"],
            true
        );
    }
    let ask = json!({"v":1,"type":"request","id":"h","method":"history","params":{"room":"big","last":40}}).to_string();
    let started = Instant::now();
    for _ in 0..200 {
        if c.send_raw(&encode(true, TEXT, ask.as_bytes(), true))
            .is_err()
        {
            break;
        }
    }
    // the client reads nothing for longer than the stall time (3 seconds here, 30 as shipped), then reads what is there:
    // the server has shut the connection, so what is read ends, in a close or a cut, and does not go on for ever
    std::thread::sleep(2 * IDLE);
    c.set_timeout(Duration::from_secs(10));
    let mut bytes = 0usize;
    let ended = loop {
        match c.got() {
            Got::Frame(Frame::Text(t)) => bytes += t.len(),
            Got::Frame(Frame::Ping(p)) => {
                // answering pings is how a client that is alive but not reading would stay: it is not reading, so it
                // does not answer them either way; the point is that the connection is over
                let _ = c.send_raw(&encode(true, PONG, &p, true));
            }
            Got::Frame(_) => {}
            Got::Eof => break true,
            Got::Quiet => break false,
        }
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "still sending after {bytes} bytes"
        );
    };
    assert!(ended, "the connection was still open after {bytes} bytes");
    assert!(
        bytes < 200 * 40 * 20_000,
        "all of it was delivered: the server never stalled"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// shared connection limits, and nothing left behind

fn open_files(pid: u32) -> Option<usize> {
    let out = std::process::Command::new("lsof")
        .args(["-nP", "-p", &pid.to_string()])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).lines().count())
}

#[test]
fn every_way_a_connection_can_end_gives_back_its_slot_its_thread_and_its_descriptor() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = plain_server(&["--max-connections", "2"]);
    let pid = rig.server.child.id();
    // one warm-up, so the baseline is a server that has served a connection
    drop(open(&rig));
    let before = open_files(pid);
    for round in 0..4 {
        // the client closes properly
        let mut c = open(&rig);
        c.close(1000);
        ends_with_close(&mut c, 1000);
        // the client drops the connection with no close
        drop(open(&rig));
        // the client breaks the protocol
        let mut c = open(&rig);
        c.send(true, BINARY, b"x");
        ends_with_close(&mut c, 1003);
        // the client sends a frame header and goes away
        let mut c = open(&rig);
        c.send_raw(&header_only(TEXT, 100)).unwrap();
        drop(c);
        // the client is silent and is closed by the server's idle time (once, it takes seconds)
        if round == 0 {
            let mut c = open(&rig);
            let frames = c.until_end();
            assert!(
                matches!(frames.last(), Some(Frame::Close(Some(1001), _))),
                "{frames:?}"
            );
        }
    }
    // with a cap of two, two open connections fill it and a third is told it is busy: the cap is the HTTP listener's, and
    // all that came and went above gave its slot back (or this would already have been refused)
    let (a, b) = (open(&rig), open(&rig));
    let busy = rig.net.request(("GET", "/v1/stream"), &[], b"");
    let status = read_head(&mut std::io::BufReader::new(busy)).0;
    assert_eq!(status, 503, "a third connection over a cap of two");
    drop((a, b));
    if let (Some(before), Some(after)) = (before, open_files(pid)) {
        assert!(
            after <= before + 6,
            "descriptors went from {before} to {after}"
        );
    }
}
