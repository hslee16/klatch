//! How a `GET /v1/stream` ends, and the caps over TLS (#1621, #1517). A session that panics must still be cut and give its
//! slots back while its client stays connected (the server is started with `KLATCH_TEST_STREAM_PANIC`, a knob of a debug
//! build only), and the per-member cap and the maximum age work behind mutual TLS as they do on loopback. Every read has a
//! deadline, so a regression fails an assertion and never hangs a test.

mod tls_common;

use std::io::{Read, Write};
use std::time::{Duration, Instant};
use tls_common::wire::*;
use tls_common::*;

const SLACK: Duration = Duration::from_secs(10);

fn stream_request(member: &str, token: &str) -> String {
    format!(
        "GET /v1/stream HTTP/1.1\r\nHost: localhost\r\nX-Klatch-Member: {member}\r\nAuthorization: Bearer {token}\r\n\r\n"
    )
}

/// What the server answers to `head`, until the end of the connection or for at most `within`: a refusal ends the
/// connection, but a stream wrongly let in never does. Returns the bytes and whether the connection ended.
fn answer(net: &Net, head: &str, within: Duration) -> (String, bool) {
    let mut c = net.connect();
    c.set_read_timeout(Duration::from_millis(500));
    c.write_all(head.as_bytes()).unwrap();
    c.flush().unwrap();
    read_until_end(&mut c, within)
}

fn read_until_end(c: &mut Conn, within: Duration) -> (String, bool) {
    let mut out = Vec::new();
    let mut buf = [0u8; 512];
    let until = Instant::now() + within;
    let mut ended = false;
    while Instant::now() < until {
        match c.read(&mut buf) {
            Ok(0) => {
                ended = true;
                break;
            }
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => {
                ended = true;
                break;
            }
        }
    }
    (String::from_utf8_lossy(&out).into_owned(), ended)
}

/// The status of a stream request, retried while the server answers 503 (a place given back is seen a moment after the
/// client or the server lets go); bounded by `SLACK`. The connection of a 200 is returned too, so it stays open.
fn status_soon(net: &Net, member: &str, token: &str) -> (u16, Option<Conn>) {
    let until = Instant::now() + SLACK;
    loop {
        let mut c = net.connect();
        c.set_read_timeout(Duration::from_secs(5));
        c.write_all(stream_request(member, token).as_bytes())
            .unwrap();
        c.flush().unwrap();
        let mut r = std::io::BufReader::new(c);
        let (status, _) = read_head(&mut r);
        if status != 503 || Instant::now() >= until {
            return (status, Some(r.into_inner()));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn a_session_that_panics_is_cut_and_gives_its_member_slot_back_while_its_client_stays_connected() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let tokens = [issue_credential(&dir, "alice")];
    // one place of the member: a leak of it refuses the second stream (the global slot of the plain listener is not
    // checked: serve_concurrent keeps the slot of a handler that panics, a separate gap, ticketed)
    let args: Vec<String> = ["--max-connections", "4", "--stream-max-per-member", "1"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let server = Server::started_env(&dir, "http", &args, &[("KLATCH_TEST_STREAM_PANIC", "1")]);
    let net = Net {
        addr: server.addr,
        tls: None,
    };
    // the client sends its request and then stays connected: it does not drop the connection
    let mut c = net.connect();
    c.set_read_timeout(Duration::from_millis(500));
    c.write_all(stream_request("alice", &tokens[0]).as_bytes())
        .unwrap();
    c.flush().unwrap();
    let started = Instant::now();
    let (text, ended) = read_until_end(&mut c, Duration::from_secs(6));
    assert!(
        ended,
        "the stream of a session that panicked was still open after {:?}: {text:?}",
        started.elapsed()
    );
    assert!(
        dir.log().contains("internal error at"),
        "the forced panic did not happen:\n{}",
        dir.log()
    );
    // the log names the place crate-relative and never the message (#1645; the trimming of an absolute path is pinned by the
    // unit tests of `panic_log`, since cargo gives a workspace crate a relative path already)
    let log = dir.log();
    assert!(
        log.contains("internal error at klatch-server/src/wire_stream_run.rs:"),
        "{log}"
    );
    assert!(
        !log.contains(env!("CARGO_MANIFEST_DIR")),
        "a build path in the log:\n{log}"
    );
    assert!(
        !log.contains("forced panic"),
        "the panic message in the log:\n{log}"
    );
    // the dead session is not reachable: the documented not-found answer, at once (#1643; the entry itself is pinned by
    // the unit tests of `wire_sessions`, since a dropped receiver answers the same 404)
    let id = text
        .lines()
        .find_map(|l| l.strip_prefix("X-Klatch-Session: "))
        .expect("the head names the session")
        .trim()
        .to_string();
    let bearer = format!("Bearer {}", tokens[0]);
    let status = status_of(
        &net,
        ("POST", "/v1/frames"),
        &[
            ("X-Klatch-Member", "alice"),
            ("Authorization", &bearer),
            ("X-Klatch-Session", &id),
        ],
        b"{\"v\":1,\"type\":\"request\",\"id\":\"r1\",\"method\":\"join\",\"params\":{\"room\":\"conf\"}}\n",
    );
    assert_eq!(status, 404, "a frame for a session that panicked");
    // the first client is still connected; the slots are back, so a new stream is let in (and is cut in its turn)
    let (status, second) = status_soon(&net, "alice", &tokens[0]);
    assert_eq!(status, 200, "the member slot leaked through the panic");
    drop(second);
    drop(c);
}

#[test]
fn twenty_session_panics_leave_one_hook_line_and_one_handler_line_in_the_log() {
    // every stream of this server panics: the hook runs for each and so does the catch at the top of the handler (#1645)
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let tokens = [issue_credential(&dir, "alice")];
    let server = Server::started_env(&dir, "http", &[], &[("KLATCH_TEST_STREAM_PANIC", "1")]);
    let net = Net {
        addr: server.addr,
        tls: None,
    };
    for i in 0..20 {
        let mut c = net.connect();
        c.set_read_timeout(Duration::from_millis(500));
        c.write_all(stream_request("alice", &tokens[0]).as_bytes())
            .unwrap();
        c.flush().unwrap();
        let (text, ended) = read_until_end(&mut c, Duration::from_secs(6));
        assert!(ended, "stream {i} was not cut: {text:?}");
    }
    // the last panic may still be on its way to the log: wait (bounded) for the handler's line
    let until = Instant::now() + SLACK;
    while !dir.log().contains("handler panicked") && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    let log = dir.log();
    let count = |needle: &str| log.lines().filter(|l| l.contains(needle)).count();
    assert_eq!(count("internal error"), 1, "hook lines:\n{log}");
    assert_eq!(count("handler panicked"), 1, "handler lines:\n{log}");
    assert!(!log.contains("forced panic"), "{log}");
}

#[test]
fn a_server_without_short_limits_ignores_the_panic_knob() {
    // the knob needs the short-limits one as well (a debug build, as `cargo test` builds): a server run as an operator
    // runs it, with the panic variable in its environment, serves its stream and logs no panic
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let tokens = [issue_credential(&dir, "alice")];
    let server = Server::started_with(
        &dir,
        "http",
        &[],
        false,
        &[("KLATCH_TEST_STREAM_PANIC", "1")],
    );
    let net = Net {
        addr: server.addr,
        tls: None,
    };
    let mut sse = Sse::open(&net, "alice", &tokens[0]);
    sse.r.get_ref().set_read_timeout(Duration::from_secs(10));
    let r = sse.request("join", serde_json::json!({"room": "conf"}));
    assert_eq!(r["ok"], true, "{r}");
    let log = dir.log();
    assert!(
        !log.contains("internal error at") && !log.contains("handler panicked"),
        "the panic knob worked without short limits:\n{log}"
    );
}

#[test]
fn over_tls_a_member_at_her_cap_gets_the_refusal_a_full_server_gives_and_a_slot_given_back_is_usable()
 {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = tls_server_with(&["--stream-max-per-member", "2"]);
    let (a1, a2) = (
        Sse::open(&rig.net, "alice", &rig.tokens[0]),
        Sse::open(&rig.net, "alice", &rig.tokens[0]),
    );
    let (refused, ended) = answer(
        &rig.net,
        &stream_request("alice", &rig.tokens[0]),
        Duration::from_secs(5),
    );
    assert!(ended, "the refusal did not end the connection: {refused:?}");
    assert!(refused.starts_with("HTTP/1.1 503"), "{refused:?}");
    // bob is not held out by alice's two
    let _b = Sse::open(&rig.net, "bob", &rig.tokens[1]);
    // the same bytes as a loopback server at its connection cap gives anyone (a full TLS server says nothing, so the
    // comparison is with the plain refusal that the cap per member copies: at the connection cap it closes before the
    // handshake, at the session cap after it, and with no bytes either way)
    let full = tls_common::ws::plain_server(&["--max-connections", "1"]);
    let _holder = Sse::open(&full.net, "bob", &full.tokens[1]);
    let (when_full, _) = answer(
        &full.net,
        &stream_request("alice", &full.tokens[0]),
        Duration::from_secs(5),
    );
    assert_eq!(refused, when_full);
    // a place given back is usable again
    drop(a1);
    let (status, again) = status_soon(&rig.net, "alice", &rig.tokens[0]);
    assert_eq!(status, 200, "alice could not reconnect after closing one");
    drop(again);
    drop(a2);
}

#[test]
fn over_tls_a_stream_is_cut_at_the_maximum_age_and_its_slot_comes_back() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let rig = tls_server_with(&["--stream-max-per-member", "2", "--stream-max-age-secs", "3"]);
    let held = Sse::open(&rig.net, "alice", &rig.tokens[0]);
    let mut c = held.r;
    c.get_ref().set_read_timeout(Duration::from_millis(500));
    let started = Instant::now();
    // read to the end of the stream, by the clock
    let mut sink = Vec::new();
    let mut buf = [0u8; 512];
    let mut ended = false;
    while started.elapsed() < SLACK {
        match c.read(&mut buf) {
            Ok(0) => {
                ended = true;
                break;
            }
            Ok(n) => sink.extend_from_slice(&buf[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => {
                ended = true;
                break;
            }
        }
    }
    assert!(ended, "still open after {:?}", started.elapsed());
    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "ended early: {:?}",
        started.elapsed()
    );
    // the slot came back with the end of the stream (the cap is two: fill it, so a leak of the first is seen)
    let (s1, c1) = status_soon(&rig.net, "alice", &rig.tokens[0]);
    let (s2, c2) = status_soon(&rig.net, "alice", &rig.tokens[0]);
    assert_eq!(
        (s1, s2),
        (200, 200),
        "a slot of the cut stream was not given back"
    );
    drop((c1, c2));
}
