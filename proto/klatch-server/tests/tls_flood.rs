//! A peer with no certificate cannot starve the clients that have one (#1493): connections still in the TLS handshake
//! have a pool of their own, apart from the session cap; one source address holds only a few of them; and every way a
//! handshake can end gives its place back. Certificates are throwaway (`tls_common`). Every wait has a deadline, so a
//! regression fails a test and never hangs it.

mod tls_common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tls_common::wire::*;
use tls_common::*;

/// Wait up to ten seconds until `want` of these sockets have been closed by the server (an event, never a time to guess).
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

/// An event stream opened over a fresh connection and held open (a session that stays), or the reason it was refused. A
/// stream is held by the server until the client lets go, so the place it takes is held for as long as the test says.
fn open_session(t: &TlsRig, n: usize) -> std::io::Result<Tls> {
    let (member, token) = (["alice", "bob"][n % 2], &t.tokens[n % 2]);
    let mut tls = tls_connect(t.server.addr, t.net.tls.as_ref().unwrap())?;
    tls.sock.set_read_timeout(Some(Duration::from_secs(10)))?;
    let request = format!(
        "GET /v1/stream HTTP/1.1\r\nHost: x\r\nX-Klatch-Member: {member}\r\nAuthorization: Bearer {token}\r\n\r\n"
    );
    tls.write_all(request.as_bytes())?;
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        tls.read_exact(&mut byte)?;
        head.push(byte[0]);
    }
    if head.starts_with(b"HTTP/1.1 200") {
        Ok(tls)
    } else {
        Err(std::io::Error::other("not served"))
    }
}

/// Serve a valid client, retrying until a deadline (a place that the server gives back after a connection ended is given
/// back by another thread, a moment later). A leak is a place that never comes back, and fails at the deadline.
fn served_within(t: &TlsRig, secs: u64) -> Result<(), String> {
    let until = Instant::now() + Duration::from_secs(secs);
    loop {
        match get_status(t.server.addr, t.net.tls.as_ref().unwrap(), "/nope") {
            Ok(404) => return Ok(()),
            other if Instant::now() >= until => return Err(format!("{other:?}")),
            _ => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

#[test]
fn stalled_handshakes_do_not_take_the_places_of_sessions() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    // four connections in all; three silent sockets are in the handshake (a pool of their own), and four valid clients
    // must still hold four sessions. Before the pool was separate the silent sockets took three of the four places.
    let t = tls_server_with(&[
        "--max-connections",
        "4",
        "--max-handshakes",
        "4",
        "--handshakes-per-source",
        "4",
        "--handshake-secs",
        "60",
    ]);
    let flood: Vec<TcpStream> = (0..3)
        .map(|_| TcpStream::connect(t.server.addr).unwrap())
        .collect();
    let mut sessions = Vec::new();
    for i in 0..4 {
        match open_session(&t, i) {
            Ok(s) => sessions.push(s),
            Err(e) => panic!("valid client {i} was not served while 3 sockets stalled: {e}"),
        }
    }
    drop(flood);
}

#[test]
fn a_session_over_the_session_cap_is_closed_and_the_stalled_do_not_count_toward_it() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let t = tls_server_with(&[
        "--max-connections",
        "2",
        "--max-handshakes",
        "2",
        "--handshakes-per-source",
        "2",
        "--handshake-secs",
        "60",
    ]);
    let a = open_session(&t, 0).expect("first session");
    let b = open_session(&t, 1).expect("second session");
    assert!(
        open_session(&t, 2).is_err(),
        "a third session was served over a cap of two"
    );
    drop(a);
    drop(b);
}

#[test]
fn one_source_holds_at_most_its_share_of_the_handshake_pool_and_the_rest_are_closed_unread() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    // a pool of four, two per source: three silent sockets from this one address means one is closed, and the pool has room
    let t = tls_server_with(&[
        "--max-connections",
        "8",
        "--max-handshakes",
        "4",
        "--handshakes-per-source",
        "2",
        "--handshake-secs",
        "60",
    ]);
    let silent: Vec<TcpStream> = (0..3)
        .map(|_| TcpStream::connect(t.server.addr).unwrap())
        .collect();
    assert_eq!(
        wait_until_closed(&silent, 1),
        1,
        "exactly one of three silent sockets from one address is over its share"
    );
    // the share is this address's: the same address cannot take a third place
    assert!(
        get_status(t.server.addr, t.net.tls.as_ref().unwrap(), "/nope").is_err(),
        "a third handshake from one address was admitted"
    );
    // when the silent sockets end the places come back and the address is served again
    drop(silent);
    served_within(&t, 10).expect("the places were not given back after the clients reset");
}

#[test]
fn every_way_a_handshake_can_end_gives_its_place_back() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    // one place for one source: any leak makes the next valid client fail at its deadline
    let t = tls_server_with(&[
        "--max-connections",
        "4",
        "--max-handshakes",
        "2",
        "--handshakes-per-source",
        "1",
        "--handshake-secs",
        "1",
    ]);
    for round in 0..3 {
        // a handshake that completes
        served_within(&t, 10).unwrap_or_else(|e| panic!("round {round} complete: {e}"));
        // a handshake that fails (not TLS at all)
        let mut junk = TcpStream::connect(t.server.addr).unwrap();
        let _ = junk.write_all(b"GET / HTTP/1.1\r\n\r\n");
        wait_until_closed(std::slice::from_ref(&junk), 1);
        served_within(&t, 10).unwrap_or_else(|e| panic!("round {round} failure: {e}"));
        // a client that resets
        let reset = TcpStream::connect(t.server.addr).unwrap();
        drop(reset);
        served_within(&t, 10).unwrap_or_else(|e| panic!("round {round} reset: {e}"));
        // a client that goes silent until the deadline cuts it
        let silent = TcpStream::connect(t.server.addr).unwrap();
        assert_eq!(wait_until_closed(std::slice::from_ref(&silent), 1), 1);
        served_within(&t, 10).unwrap_or_else(|e| panic!("round {round} timeout: {e}"));
    }
}

#[test]
fn a_client_that_sends_a_byte_at_a_time_is_cut_at_the_deadline() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let t = tls_server_with(&[
        "--max-connections",
        "4",
        "--max-handshakes",
        "2",
        "--handshakes-per-source",
        "2",
        "--handshake-secs",
        "1",
    ]);
    let mut slow = TcpStream::connect(t.server.addr).unwrap();
    slow.set_nodelay(true).unwrap();
    let start = Instant::now();
    // a TLS record header that promises more than ever arrives, one byte every 200 ms; the loop ends at the cut or at 6 s
    let record = [0x16u8, 0x03, 0x01, 0x10, 0x00, 0x01, 0x00, 0x00, 0x00];
    let mut cut = false;
    for i in 0..30 {
        if slow.write_all(&record[i % record.len()..][..1]).is_err() {
            cut = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
        slow.set_nonblocking(true).unwrap();
        let mut one = [0u8; 1];
        if matches!((&slow).read(&mut one), Ok(0)) {
            cut = true;
            break;
        }
        slow.set_nonblocking(false).unwrap();
    }
    assert!(cut, "the server did not cut a one byte at a time client");
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "cut only after {:?} with a 1 s deadline",
        start.elapsed()
    );
}
