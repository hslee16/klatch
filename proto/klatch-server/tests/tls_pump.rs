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

/// A client config that is valid for the rig's server.
fn valid_config(t: &TlsRig) -> &std::sync::Arc<rustls::ClientConfig> {
    t.net.tls.as_ref().unwrap()
}

/// How many of these sockets the server has closed (read the end of the stream), waiting up to ten seconds for at
/// least `want`. A connection the server refuses is closed at once, so this is an event to wait for, not a time to guess.
fn wait_until_closed(sockets: &[TcpStream], want: usize) -> usize {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let closed = sockets
            .iter()
            .filter(|s| {
                s.set_nonblocking(true).unwrap();
                let mut one = [0u8; 1];
                matches!((&**s).read(&mut one), Ok(0))
            })
            .count();
        if closed >= want || Instant::now() >= until {
            return closed;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn an_honest_client_mid_handshake_is_not_evicted_by_a_flood_and_the_flood_is_refused() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    // the longest deadline there is: a minimum age of 30 s, far more than this test takes, so no machine is slow enough
    // for the age rule to apply to the honest handshake; and nothing here sleeps to order two events
    let t = tls_server_with(&[
        "--max-connections",
        "8",
        "--max-handshakes",
        "2",
        "--handshake-secs",
        "60",
    ]);
    let name = rustls::pki_types::ServerName::IpAddress(t.server.addr.ip().into());
    let mut conn = rustls::ClientConnection::new(valid_config(&t).clone(), name).unwrap();
    let mut sock = TcpStream::connect(t.server.addr).unwrap();
    sock.set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    // the honest client says hello and reads the server's answer: when the answer is here the server has the handshake
    // in progress (it has a place among them), and is waiting for the client's second flight, which is held back
    let mut hello = Vec::new();
    while conn.wants_write() {
        conn.write_tls(&mut hello).unwrap();
    }
    sock.write_all(&hello).unwrap();
    while !conn.wants_write() {
        let n = conn
            .read_tls(&mut sock)
            .expect("the server answered the hello");
        assert!(
            n > 0,
            "the server closed the honest client before the flood"
        );
        conn.process_new_packets().unwrap();
    }
    // the flood arrives while that handshake is in progress, and is refused (the cap of two is full of young handshakes)
    let flood: Vec<TcpStream> = (0..12)
        .map(|_| TcpStream::connect(t.server.addr).unwrap())
        .collect();
    let refused = wait_until_closed(&flood, 11);
    assert!(
        refused >= 11,
        "only {refused} of 12 flood sockets were refused"
    );
    // the honest client sends its second flight and is served
    while conn.is_handshaking() {
        conn.complete_io(&mut sock)
            .expect("the honest handshake was cut by the flood");
    }
    let mut tls = rustls::StreamOwned::new(conn, sock);
    tls.write_all(b"GET /nope HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    let mut status = [0u8; 12];
    tls.read_exact(&mut status).unwrap();
    assert!(
        String::from_utf8_lossy(&status).contains("404"),
        "{status:?}"
    );
    drop(flood);
}

#[test]
fn a_newcomer_over_the_cap_is_refused_while_every_handshake_is_young() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    // a 30 s minimum age: every handshake is young for the whole test
    let t = tls_server_with(&[
        "--max-connections",
        "8",
        "--max-handshakes",
        "2",
        "--handshake-secs",
        "60",
    ]);
    // three silent sockets: the cap is two, so one of them is refused, and when that is seen the cap holds two
    // handshakes that have a place (which three came first does not matter)
    let silent: Vec<TcpStream> = (0..3)
        .map(|_| TcpStream::connect(t.server.addr).unwrap())
        .collect();
    assert!(
        wait_until_closed(&silent, 1) >= 1,
        "no silent socket was refused"
    );
    // the cap is full of young handshakes: a valid client is the one that loses
    assert!(
        get_status(t.server.addr, valid_config(&t), "/nope").is_err(),
        "a newcomer evicted a young handshake"
    );
    drop(silent);
}

#[test]
fn a_flood_of_silent_sockets_older_than_the_minimum_age_is_cleared_for_a_valid_client() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    // min age 2 s of a 4 s deadline; two silent sockets fill the cap
    let t = tls_server_with(&[
        "--max-connections",
        "8",
        "--max-handshakes",
        "2",
        "--handshake-secs",
        "4",
    ]);
    let silent: Vec<TcpStream> = (0..3)
        .map(|_| TcpStream::connect(t.server.addr).unwrap())
        .collect();
    assert!(
        wait_until_closed(&silent, 1) >= 1,
        "no silent socket was refused"
    );
    // past the minimum age and, on any machine that is not stalled for seconds, before the deadline: the next newcomer
    // evicts the oldest. (A machine that stalls past the deadline lets the deadline clear them instead, which serves the
    // client all the same: the test cannot fail by being slow.)
    std::thread::sleep(Duration::from_millis(2300));
    assert_eq!(
        get_status(t.server.addr, valid_config(&t), "/nope").unwrap(),
        404
    );
    drop(silent);
}

#[test]
fn the_server_closes_the_socket_within_a_couple_of_seconds_of_a_finished_answer_whatever_the_client_trickles()
 {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let t = tls_server();
    let mut tls = connect(&t);
    tls.write_all(STREAM_REQUEST).unwrap();
    tls.flush().unwrap();
    // read the whole answer, then keep sending raw bytes every 150 ms: a drain that waits for the client's silence
    // would go on for about 13 s, so the close must come from the drain's own bound
    let mut raw = tls.sock.try_clone().unwrap();
    let mut got = vec![0u8; 12];
    tls.read_exact(&mut got).unwrap();
    let answered = Instant::now();
    let closed = loop {
        if answered.elapsed() > Duration::from_secs(20) {
            panic!("the server never closed the socket");
        }
        if raw.write_all(b"x").is_err() {
            break answered.elapsed();
        }
        std::thread::sleep(Duration::from_millis(150));
    };
    assert!(closed < Duration::from_secs(6), "closed after {closed:?}");
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
