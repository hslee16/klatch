//! The pump and the pre-handshake guard of the mutual TLS listener (#108): a client that half-closes after its request
//! still gets the whole answer (TCP FIN, TLS `close_notify`, and a request whose body is still unread when the client
//! says it is done); a flood of silent sockets cannot keep a valid client out; a client that never reads is torn down by
//! the stall deadline and does not make the server spin. Certificates are throwaway (`tls_common`).

mod tls_common;

use serde_json::json;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::time::{Duration, Instant};
use tls_common::wire::*;
use tls_common::*;

/// Everything the server sends until the connection ends (a clean end, a reset or a cut; the bytes read before count).
fn read_all(tls: &mut Tls) -> String {
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    while let Ok(n) = tls.read(&mut buf) {
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    String::from_utf8_lossy(&out).into_owned()
}

const STREAM_REQUEST: &[u8] = b"GET /v1/stream HTTP/1.1\r\nHost: x\r\n\r\n";

fn connect(t: &TlsRig) -> Tls {
    tls_connect(t.server.addr, t.net.tls.as_ref().unwrap()).expect("handshake")
}

#[test]
fn a_request_then_a_tcp_half_close_still_gets_the_whole_answer() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let t = tls_server();
    let mut tls = connect(&t);
    tls.write_all(STREAM_REQUEST).unwrap();
    tls.flush().unwrap();
    tls.sock.shutdown(Shutdown::Write).unwrap();
    let got = read_all(&mut tls);
    assert!(got.starts_with("HTTP/1.1 401"), "{got:?}");
    assert!(got.ends_with("unauthorized"), "the whole body: {got:?}");
}

#[test]
fn a_request_then_a_tls_close_notify_still_gets_the_whole_answer() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let t = tls_server();
    let mut tls = connect(&t);
    tls.write_all(STREAM_REQUEST).unwrap();
    tls.conn.send_close_notify();
    tls.flush().unwrap();
    let got = read_all(&mut tls);
    assert!(got.starts_with("HTTP/1.1 401"), "{got:?}");
    assert!(got.ends_with("unauthorized"), "the whole body: {got:?}");
}

#[test]
fn pipelined_requests_sent_with_the_close_are_read_and_the_first_is_answered() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let t = tls_server();
    let mut tls = connect(&t);
    // two requests and the end of the stream in one write: the server reads the first, answers it, and closes
    let two = [STREAM_REQUEST, STREAM_REQUEST].concat();
    tls.write_all(&two).unwrap();
    tls.conn.send_close_notify();
    tls.flush().unwrap();
    let got = read_all(&mut tls);
    assert!(got.starts_with("HTTP/1.1 401"), "{got:?}");
}

#[test]
fn a_request_body_still_unread_when_the_client_closes_is_not_discarded() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let t = tls_server();
    let mut alice = Sse::open(&t.net, "alice", &t.tokens[0]);
    assert_eq!(alice.recv().unwrap()["type"], "hello");
    // a POST with a credential, its body and the close in one flight: the frame must reach the session
    let frame =
        json!({"v":1,"type":"request","id":"half","method":"join","params":{"room":"half"}});
    let body = format!("{frame}\n");
    let request = format!(
        "POST /v1/frames HTTP/1.1\r\nHost: x\r\nX-Klatch-Member: alice\r\nAuthorization: Bearer {}\r\nX-Klatch-Session: {}\r\nContent-Length: {}\r\n\r\n{body}",
        t.tokens[0],
        alice.session,
        body.len()
    );
    let mut tls = connect(&t);
    tls.write_all(request.as_bytes()).unwrap();
    tls.conn.send_close_notify();
    tls.flush().unwrap();
    let got = read_all(&mut tls);
    assert!(got.starts_with("HTTP/1.1 202"), "{got:?}");
    let response = loop {
        let f = alice.recv().expect("the response to the half-closed POST");
        if f["type"] == "response" && f["id"] == "half" {
            break f;
        }
    };
    assert_eq!(response["ok"], true, "{response}");
}

// ---------------------------------------------------------------------------------------------------------------
// the pre-handshake guard

#[test]
fn a_flood_of_silent_sockets_cannot_keep_a_valid_client_out() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    // a long handshake deadline, so that only the guard (not the clock) can make room
    // (a small connection cap, so that without the guard the silent sockets would fill every slot)
    let t = tls_server_with(&[
        "--max-connections",
        "8",
        "--max-handshakes",
        "4",
        "--handshake-secs",
        "60",
    ]);
    let silent: Vec<TcpStream> = (0..24)
        .map(|_| TcpStream::connect(t.server.addr).unwrap())
        .collect();
    let leaf = &t.net;
    // the server has taken them all in (evicted handshakes are closed and their threads gone) before the client comes
    std::thread::sleep(Duration::from_millis(400));
    for _ in 0..3 {
        assert_eq!(
            get_status(t.server.addr, leaf.tls.as_ref().unwrap(), "/nope").unwrap(),
            404,
            "a valid client is served while {} silent sockets are open",
            silent.len()
        );
    }
}

#[test]
fn a_socket_that_never_finishes_the_handshake_is_closed_at_its_deadline() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let t = tls_server_with(&["--handshake-secs", "1"]);
    let mut silent = TcpStream::connect(t.server.addr).unwrap();
    silent
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let started = Instant::now();
    let mut buf = [0u8; 16];
    let ended = silent.read(&mut buf);
    assert!(
        matches!(ended, Ok(0) | Err(_))
            && !matches!(&ended, Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
        "{ended:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
}

// ---------------------------------------------------------------------------------------------------------------
// the stall deadline

/// The CPU time a process has used so far, in seconds (`ps -o cputime`, `[H:]M:SS.cc`).
fn cpu_seconds(pid: u32) -> f64 {
    let out = std::process::Command::new("/bin/ps")
        .args(["-o", "cputime=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    text.split(':')
        .map(|p| {
            p.parse::<f64>()
                .unwrap_or_else(|_| panic!("cputime {text:?}"))
        })
        .fold(0.0, |acc, p| acc * 60.0 + p)
}

#[test]
fn a_client_that_never_reads_is_torn_down_by_the_stall_deadline_and_the_server_does_not_spin() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let t = tls_server_with(&["--stall-secs", "3"]);
    let pid = t.server.child.id();
    // a room with about a megabyte of history, from a client that does read
    let mut poster = Sse::open(&t.net, "alice", &t.tokens[0]);
    poster.recv().unwrap();
    assert_eq!(poster.request("join", json!({"room": "big"}))["ok"], true);
    let body = "x".repeat(30_000);
    for i in 0..40 {
        poster.post("big", &format!("{i} {body}"));
    }
    // the client that stops reading: it subscribes from the start (the whole history is the answer), then reads nothing
    let mut idle = Sse::open(&t.net, "bob", &t.tokens[1]);
    idle.recv().unwrap();
    assert_eq!(
        idle.post_frame(
            &json!({"v":1,"type":"request","id":"s","method":"join","params":{"room":"big"}})
        ),
        202
    );
    let before = cpu_seconds(pid);
    let started = Instant::now();
    assert_eq!(idle.post_frame(&json!({"v":1,"type":"request","id":"sub","method":"subscribe","params":{"room":"big","from_seq":0}})), 202);
    std::thread::sleep(Duration::from_secs(2));
    let spent = cpu_seconds(pid) - before;
    assert!(
        spent < 1.5,
        "the server used {spent:.2} s of CPU in 2 s while a client read nothing"
    );
    // well past the 3 second deadline: the connection is gone, so what was buffered ends in an end. Without the deadline
    // the stream would go on (heartbeats), so the read loop is bounded by a clock of its own and fails by assertion.
    std::thread::sleep(Duration::from_secs(4));
    idle.r.get_ref().set_read_timeout(Duration::from_secs(5));
    let give_up = Instant::now() + Duration::from_secs(25);
    let mut sink = [0u8; 65536];
    let ended = loop {
        if Instant::now() > give_up {
            break Err(std::io::ErrorKind::TimedOut);
        }
        match std::io::Read::read(&mut idle.r, &mut sink) {
            Ok(0) => break Ok(()),
            Ok(_) => {}
            Err(e) => break Err(e.kind()),
        }
    };
    assert!(
        !matches!(
            ended,
            Err(std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)
        ),
        "the connection was never torn down: {ended:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(40),
        "{:?}",
        started.elapsed()
    );
    // and the server is still serving
    assert_eq!(
        get_status(t.server.addr, t.net.tls.as_ref().unwrap(), "/nope").unwrap(),
        404
    );
}
