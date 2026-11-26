//! The pump of the mutual TLS listener (#108, ticket #1494) at its limits, through the real https listener (throwaway
//! certificates from `tls_common`, loopback, ephemeral ports, a temporary store): a body of exactly the 1 MiB limit is
//! accepted and one byte over is refused with the `413`; output many times the pump's window arrives complete and in
//! order; a client that reads a little and sleeps does not make the server buffer without bound, and one that then stops
//! reading is cut by the stall deadline; a client that half-closes after a complete request gets the whole answer, and one
//! that half-closes in the middle of a request gets a prompt, clean end and no hang.
//!
//! Every read and every wait here has a deadline, so a regression fails by assertion within seconds and never hangs.

mod tls_common;

use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::Shutdown;
use std::time::{Duration, Instant};
use tls_common::wire::*;
use tls_common::*;

/// The largest body the wire takes (`loopback::Limits::default().max_body`); one byte more is `413` before it is read.
const BODY_LIMIT: usize = 1024 * 1024;

/// A message body is at most 32768 bytes (the room's own limit); frames of this size make a request near the limit
/// out of valid frames, so the session accepts all of them.
const FRAME_BYTES: usize = 30_000;

/// How long any single read waits in these tests before the test fails.
const READ_DEADLINE: Duration = Duration::from_secs(15);

/// The pump's window in each direction, read from the source that defines it so that the bounds below follow it.
fn window() -> usize {
    let source = include_str!("../src/tls_bridge.rs");
    let line = source
        .lines()
        .find(|l| l.starts_with("pub const WINDOW: usize"))
        .expect("tls_bridge.rs names WINDOW");
    let expr = line.split('=').nth(1).unwrap().trim().trim_end_matches(';');
    expr.split('*')
        .map(|n| n.trim().parse::<usize>().expect("a number"))
        .product()
}

fn connect(t: &TlsRig) -> Tls {
    let tls = tls_connect(t.server.addr, t.net.tls.as_ref().unwrap()).expect("handshake");
    tls.sock.set_read_timeout(Some(READ_DEADLINE)).unwrap();
    tls.sock.set_write_timeout(Some(READ_DEADLINE)).unwrap();
    tls
}

/// How a read of the server's answer ended.
#[derive(Debug, PartialEq)]
enum End {
    /// `close_notify` or the end of the stream: a clean end.
    Clean,
    /// A read waited past [`READ_DEADLINE`]: the server hung.
    Hung,
    /// A reset or a cut without `close_notify`.
    Cut,
}

/// Everything the server sends until the connection ends, and how it ended.
fn read_all(tls: &mut Tls) -> (Vec<u8>, End) {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match tls.read(&mut buf) {
            Ok(0) => return (out, End::Clean),
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return (out, End::Hung);
            }
            Err(_) => return (out, End::Cut),
        }
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A `POST /v1/frames` head for the member's own session.
fn post_head(sse: &Sse, token: &str, length: usize) -> String {
    format!(
        "POST /v1/frames HTTP/1.1\r\nHost: x\r\nX-Klatch-Member: {}\r\nAuthorization: Bearer {token}\r\nX-Klatch-Session: {}\r\nContent-Length: {length}\r\n\r\n",
        sse.member, sse.session
    )
}

/// One request body of `total` bytes made of valid `post` frames, one per line, and what each carries.
struct Batch {
    bytes: Vec<u8>,
    ids: Vec<String>,
    bodies: Vec<String>,
}

fn frame_line(id: &str, room: &str, body: &str) -> String {
    format!(
        "{}\n",
        json!({"v":1,"type":"request","id":id,"method":"post","params":{"room":room,"body":body}})
    )
}

fn batch_of_size(prefix: &str, room: &str, total: usize) -> Batch {
    let count = total.div_ceil(FRAME_BYTES);
    let each = total / count;
    let mut batch = Batch {
        bytes: Vec::new(),
        ids: Vec::new(),
        bodies: Vec::new(),
    };
    for i in 0..count {
        let id = format!("{prefix}{i:03}");
        let size = if i + 1 == count {
            total - each * (count - 1)
        } else {
            each
        };
        let overhead = frame_line(&id, room, "").len();
        let marker = format!("{i:03}:");
        let body = format!("{marker}{}", "x".repeat(size - overhead - marker.len()));
        let line = frame_line(&id, room, &body);
        assert_eq!(line.len(), size);
        batch.bytes.extend_from_slice(line.as_bytes());
        batch.ids.push(id);
        batch.bodies.push(body);
    }
    assert_eq!(batch.bytes.len(), total);
    batch
}

/// A member's session that has joined `room`, with every read bounded.
fn member_in(t: &TlsRig, room: &str) -> Sse {
    let mut a = Sse::open(&t.net, "alice", &t.tokens[0]);
    a.r.get_ref().set_read_timeout(READ_DEADLINE);
    assert_eq!(a.recv().unwrap()["type"], "hello");
    assert_eq!(a.request("join", json!({"room": room}))["ok"], true);
    a
}

/// The next response frame with this id, within the read deadline.
fn response_to(a: &mut Sse, id: &str) -> Value {
    loop {
        let f = a
            .recv()
            .unwrap_or_else(|| panic!("no response {id} before the stream ended or went quiet"));
        if f["type"] == "response" && f["id"] == id {
            return f;
        }
    }
}

/// The messages of `room` from the start, read on a new stream of bob: `count` of them, in the order they arrive.
fn history(t: &TlsRig, room: &str, count: usize) -> Vec<(i64, String)> {
    let mut b = Sse::open(&t.net, "bob", &t.tokens[1]);
    b.r.get_ref().set_read_timeout(READ_DEADLINE);
    b.recv().unwrap();
    assert_eq!(b.request("join", json!({"room": room}))["ok"], true);
    assert_eq!(
        b.request("subscribe", json!({"room": room, "from_seq": 0}))["ok"],
        true
    );
    let mut got = Vec::new();
    while got.len() < count {
        let f = b.recv().expect("the stream ended before the last message");
        if f["type"] == "message" {
            got.push((
                f["seq"].as_i64().unwrap(),
                f["body"].as_str().unwrap().to_string(),
            ));
        }
    }
    got
}

// ---------------------------------------------------------------------------------------------------------------
// (a) the body limit, and output much larger than the window

#[test]
fn a_body_of_exactly_the_limit_is_accepted_and_every_frame_in_it_arrives_whole_and_in_order() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let t = tls_server();
    let mut alice = member_in(&t, "big");
    let batch = batch_of_size("big", "big", BODY_LIMIT);
    let mut tls = connect(&t);
    tls.write_all(post_head(&alice, &t.tokens[0], batch.bytes.len()).as_bytes())
        .unwrap();
    tls.write_all(&batch.bytes).unwrap();
    tls.flush().unwrap();
    let (answer, end) = read_all(&mut tls);
    let answer = text(&answer);
    assert_eq!(end, End::Clean, "{answer:?}");
    assert!(answer.starts_with("HTTP/1.1 202"), "{answer:?}");
    // every frame was answered, in order, and none was refused
    let mut seqs = Vec::new();
    for id in &batch.ids {
        let r = response_to(&mut alice, id);
        assert_eq!(r["ok"], true, "{r}");
        seqs.push(r["result"]["seq"].as_i64().unwrap());
    }
    assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1), "{seqs:?}");
    // and the messages are in the room with every byte of every body, in order
    let got = history(&t, "big", batch.bodies.len());
    let got_bodies: Vec<&String> = got.iter().map(|(_, b)| b).collect();
    assert_eq!(got_bodies, batch.bodies.iter().collect::<Vec<_>>());
}

#[test]
fn a_body_one_byte_over_the_limit_is_refused_with_413_whether_or_not_the_body_is_sent() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let t = tls_server();
    let alice = member_in(&t, "big");
    // the declared length alone is enough: no body follows
    let mut tls = connect(&t);
    tls.write_all(post_head(&alice, &t.tokens[0], BODY_LIMIT + 1).as_bytes())
        .unwrap();
    tls.flush().unwrap();
    let (answer, end) = read_all(&mut tls);
    let answer = text(&answer);
    assert_eq!(end, End::Clean, "{answer:?}");
    assert!(answer.starts_with("HTTP/1.1 413"), "{answer:?}");
    assert!(answer.ends_with("body too large"), "{answer:?}");
    // and with the start of the over-long body sent after the head (less than the pump reads out after an answer, 256 KiB):
    // the same answer, read to its end. (A client that sends the whole megabyte after a refusal can have its writes fail
    // once the server has closed; the answer is still in its socket, but a client library may report the write error
    // first, so that is not asserted here.)
    let batch = batch_of_size("over", "big", BODY_LIMIT + 1);
    let mut tls = connect(&t);
    tls.write_all(post_head(&alice, &t.tokens[0], batch.bytes.len()).as_bytes())
        .unwrap();
    tls.write_all(&batch.bytes[..200_000]).unwrap();
    tls.flush().unwrap();
    let (answer, end) = read_all(&mut tls);
    let answer = text(&answer);
    assert_eq!(end, End::Clean, "{answer:?}");
    assert!(answer.starts_with("HTTP/1.1 413"), "{answer:?}");
    assert!(answer.ends_with("body too large"), "{answer:?}");
}

#[test]
fn output_many_times_the_window_arrives_complete_and_in_order() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let t = tls_server();
    let mut alice = member_in(&t, "flow");
    // enough history that the answer to one subscribe is thirty windows
    let count = (30 * window()).div_ceil(FRAME_BYTES);
    let bodies: Vec<String> = (0..count)
        .map(|i| format!("{i:05}:{}", "y".repeat(FRAME_BYTES - 6)))
        .collect();
    for body in &bodies {
        alice.post("flow", body);
    }
    let got = history(&t, "flow", count);
    assert!(got.windows(2).all(|w| w[1].0 == w[0].0 + 1), "seq order");
    let got_bodies: Vec<&String> = got.iter().map(|(_, b)| b).collect();
    assert_eq!(got_bodies, bodies.iter().collect::<Vec<_>>());
}

// ---------------------------------------------------------------------------------------------------------------
// (b) a slow reader

/// The resident memory of a process in bytes (`ps -o rss=`, in KiB).
fn rss_bytes(pid: u32) -> usize {
    let out = std::process::Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let kib: usize = text(&out.stdout).trim().parse().expect("ps rss");
    kib * 1024
}

/// Posts a room takes in a minute (the room's own rate limit is 100), so history is spread over several rooms.
const PER_ROOM: usize = 80;

/// About `volume` bytes of history in several rooms, posted by a client that reads: the rooms.
fn big_history(t: &TlsRig, volume: usize) -> Vec<String> {
    let count = volume.div_ceil(FRAME_BYTES);
    let rooms: Vec<String> = (0..count.div_ceil(PER_ROOM))
        .map(|r| format!("hist{r}"))
        .collect();
    let mut poster = member_in(t, "hist-first");
    for (r, room) in rooms.iter().enumerate() {
        assert_eq!(poster.request("join", json!({"room": room}))["ok"], true);
        for i in (r * PER_ROOM)..((r + 1) * PER_ROOM).min(count) {
            poster.post(room, &format!("{i:05}:{}", "z".repeat(FRAME_BYTES - 6)));
        }
    }
    rooms
}

/// A new stream of bob, and in ONE request (so that a stream with a short stall time is never quiet for long) the joins
/// and the subscribes for the whole history of every room: the answer is the volume of the history, sent at once.
fn reader_of_all(t: &TlsRig, rooms: &[String]) -> Sse {
    let mut sse = Sse::open(&t.net, "bob", &t.tokens[1]);
    sse.r.get_ref().set_read_timeout(READ_DEADLINE);
    sse.recv().unwrap();
    let mut lines = String::new();
    for (i, room) in rooms.iter().enumerate() {
        for (kind, method, params) in [
            ("join", "join", json!({"room": room})),
            ("sub", "subscribe", json!({"room": room, "from_seq": 0})),
        ] {
            let frame = json!({"v":1,"type":"request","id":format!("{kind}{i}"),"method":method,"params":params});
            lines.push_str(&format!("{frame}\n"));
        }
    }
    let h = [
        ("X-Klatch-Member", sse.member.as_str()),
        ("Authorization", sse.bearer.as_str()),
        ("X-Klatch-Session", sse.session.as_str()),
    ];
    let status = status_of(&sse.net, ("POST", "/v1/frames"), &h, lines.as_bytes());
    assert_eq!(status, 202);
    sse
}

#[test]
fn a_client_that_reads_a_kilobyte_and_sleeps_does_not_make_the_server_buffer_without_bound_and_is_not_cut()
 {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    // a stall time well over the test's length: this client is slow, not stalled
    let t = tls_server_with(&["--stall-secs", "60"]);
    let pid = t.server.child.id();
    let volume = 12 * 1024 * 1024;
    let rooms = big_history(&t, volume);
    let before = rss_bytes(pid);
    let mut slow = reader_of_all(&t, &rooms);
    // read 1 KiB, sleep, again, for two seconds
    let (mut peak, mut read) = (before, 0usize);
    let until = Instant::now() + Duration::from_secs(2);
    let mut kib = [0u8; 1024];
    while Instant::now() < until {
        match Read::read(&mut slow.r.get_mut(), &mut kib) {
            Ok(0) => panic!("the server cut a slow reader after {read} bytes"),
            Ok(n) => read += n,
            Err(e) => panic!("the slow reader's read failed after {read} bytes: {e}"),
        }
        std::thread::sleep(Duration::from_millis(100));
        peak = peak.max(rss_bytes(pid));
    }
    assert!(read >= 10 * 1024, "the slow reader read only {read} bytes");
    // The handler holds what it replays (up to about the history, measured at 6 to 9 MB of 12.6 here); the pump adds at
    // most a window each way per connection. A pump that read the handler's output without the window bound would hold the
    // history a second time (measured: 21 to 23 MB), so the growth must stay under four thirds of the history.
    let grown = peak.saturating_sub(before);
    let bound = volume * 4 / 3;
    assert!(
        grown < bound,
        "the server's memory grew by {grown} bytes (bound {bound}, history {volume}, window {}) while one client read 1 KiB at a time",
        window()
    );
    // still connected and still being sent more
    assert!(Read::read(&mut slow.r.get_mut(), &mut kib).unwrap() > 0);
}

#[test]
fn a_client_that_reads_a_little_and_then_stops_is_cut_by_the_stall_deadline() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let t = tls_server_with(&["--stall-secs", "3"]);
    let volume = 12 * 1024 * 1024;
    let rooms = big_history(&t, volume);
    let mut idle = reader_of_all(&t, &rooms);
    let mut kib = [0u8; 1024];
    assert!(Read::read(&mut idle.r.get_mut(), &mut kib).unwrap() > 0);
    // then nothing is read for well over the deadline
    std::thread::sleep(Duration::from_secs(6));
    // what the connection still holds is read out; the end of it is a torn-down connection. Without the deadline the
    // stream would go on for as long as the client reads, so the loop has a clock of its own and fails by assertion
    let give_up = Instant::now() + Duration::from_secs(30);
    let mut sink = vec![0u8; 65536];
    let mut total = kib.len();
    let ended = loop {
        if Instant::now() > give_up {
            break End::Hung;
        }
        match Read::read(&mut idle.r.get_mut(), &mut sink) {
            Ok(0) => break End::Clean,
            Ok(n) => total += n,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                break End::Hung;
            }
            Err(_) => break End::Cut,
        }
    };
    assert_ne!(ended, End::Hung, "the connection was never torn down");
    assert!(
        total < volume,
        "the whole history ({volume} bytes) was delivered to a client that stopped reading: {total}"
    );
    // and the server is still serving
    assert_eq!(
        get_status(t.server.addr, t.net.tls.as_ref().unwrap(), "/nope").unwrap(),
        404
    );
}

// ---------------------------------------------------------------------------------------------------------------
// (c) half-close

#[test]
fn a_client_that_half_closes_after_a_complete_large_post_still_gets_the_whole_answer() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let t = tls_server();
    let mut alice = member_in(&t, "half");
    // a complete request of about 1 MiB (the pump carries it in many windows), then the client's FIN
    let batch = batch_of_size("half", "half", BODY_LIMIT - 1024);
    let mut tls = connect(&t);
    tls.write_all(post_head(&alice, &t.tokens[0], batch.bytes.len()).as_bytes())
        .unwrap();
    tls.write_all(&batch.bytes).unwrap();
    tls.flush().unwrap();
    tls.sock.shutdown(Shutdown::Write).unwrap();
    let (answer, end) = read_all(&mut tls);
    let answer = text(&answer);
    assert_eq!(end, End::Clean, "{answer:?}");
    assert!(answer.starts_with("HTTP/1.1 202"), "{answer:?}");
    assert!(answer.ends_with("accepted"), "the whole body: {answer:?}");
    // every frame of the half-closed request reached the session
    for id in &batch.ids {
        assert_eq!(response_to(&mut alice, id)["ok"], true);
    }
}

#[test]
fn a_client_that_half_closes_in_the_middle_of_a_request_gets_a_prompt_clean_end_and_no_hang() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let t = tls_server();
    let mut alice = member_in(&t, "mid");
    let batch = batch_of_size("mid", "mid", 4096);
    // the head promises 4096 bytes, 1000 arrive, then the client is done sending. The handler sees the end of its input
    // at once (the request deadline is 5 s, so a pump that did not pass the end on would take that long)
    let mut tls = connect(&t);
    tls.write_all(post_head(&alice, &t.tokens[0], batch.bytes.len()).as_bytes())
        .unwrap();
    tls.write_all(&batch.bytes[..1000]).unwrap();
    tls.flush().unwrap();
    tls.sock.shutdown(Shutdown::Write).unwrap();
    let started = Instant::now();
    let (answer, end) = read_all(&mut tls);
    let answer = text(&answer);
    assert_eq!(end, End::Clean, "{answer:?}");
    assert!(
        answer.is_empty() || answer.starts_with("HTTP/1.1 400"),
        "wanted no answer or a 400 for a request cut short, got {answer:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the end took {:?}",
        started.elapsed()
    );
    // the truncated frame never reached the session, and the session still works
    assert_eq!(alice.request("join", json!({"room": "mid2"}))["ok"], true);
    assert!(
        alice.extra.iter().all(|f| f["id"] != "mid000"),
        "{:?}",
        alice.extra
    );
    // half-closing in the middle of the head ends the same way
    let mut tls = connect(&t);
    tls.write_all(b"GET /v1/stream HT").unwrap();
    tls.flush().unwrap();
    tls.sock.shutdown(Shutdown::Write).unwrap();
    let started = Instant::now();
    let (answer, end) = read_all(&mut tls);
    let answer = text(&answer);
    assert_eq!(end, End::Clean, "{answer:?}");
    assert!(
        answer.is_empty() || answer.starts_with("HTTP/1.1 400"),
        "{answer:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(3));
}
