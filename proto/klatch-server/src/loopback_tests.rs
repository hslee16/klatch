//! Tests of the loopback listener (`loopback.rs`). They are in their own file because they are the one place in this
//! crate that connects a client socket (to a listener it started itself, on a loopback port chosen by the system);
//! `tests/no_network.rs` names this file for that and for nothing else.

use crate::loopback::*;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[test]
fn only_127_0_0_1_and_ipv6_loopback_are_accepted() {
    assert!(check_loopback("127.0.0.1:8080").is_ok());
    assert!(check_loopback("[::1]:8080").is_ok());
    assert!(check_loopback("127.0.0.1:0").is_ok());
    for bad in [
        "0.0.0.0:8080",
        "[::]:8080",
        "192.168.1.5:8080",
        "10.0.0.1:1",
        "127.0.0.2:8080",
        "[::ffff:127.0.0.1]:8080",
        "localhost:8080",
        "example.com:80",
        "8080",
        ":8080",
        "127.0.0.1",
        "",
    ] {
        let e = check_loopback(bad).expect_err(bad).to_string();
        assert!(e.contains("refusing"), "{bad}: {e}");
        assert!(e.contains("#157") && e.contains("#161"), "{bad}: {e}");
    }
}

#[test]
fn a_refused_address_opens_no_socket_and_an_allowed_one_binds() {
    assert!(bind_loopback("0.0.0.0:0").is_err());
    let l = bind_loopback("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    assert!(addr.ip().is_loopback());
    assert_ne!(addr.port(), 0);
}

#[test]
fn the_head_parser_reads_the_request_line_and_headers() {
    let h = parse_head(
        b"POST /github HTTP/1.1\r\nHost: x\r\nContent-Length: 12\r\nX-Hub-Signature-256: sha256=ab",
    )
    .unwrap();
    assert_eq!((h.method.as_str(), h.path.as_str()), ("POST", "/github"));
    assert_eq!(h.content_length, Some(12));
    assert_eq!(h.header("X-HUB-SIGNATURE-256"), Some("sha256=ab"));
    assert_eq!(h.header("missing"), None);
}

#[test]
fn the_head_parser_refuses_malformed_input() {
    for bad in [
        &b"GET\r\n"[..],
        b"GET /x\r\n",
        b"GET /x HTTP/1.1 extra\r\n",
        b"GET x HTTP/1.1\r\n",
        b"GET /x HTTP/2\r\n",
        b"GET /x HTTP/1.1\r\nno colon here\r\n",
        b"GET /x HTTP/1.1\r\nbad name: v\r\n",
        b"POST /x HTTP/1.1\r\nContent-Length: -1\r\n",
        b"POST /x HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 1\r\n",
        b"POST /x HTTP/1.1\r\nContent-Length: abc\r\n",
        b"\xff\xfe /x HTTP/1.1\r\n",
    ] {
        assert!(
            matches!(parse_head(bad), Err(Fault::Malformed(_))),
            "{:?}",
            String::from_utf8_lossy(bad)
        );
    }
    assert_eq!(
        parse_head(b"POST /x HTTP/1.1\r\nTransfer-Encoding: chunked"),
        Err(Fault::Unsupported)
    );
}

/// A listener with a recording handler for the tests below: `/github` and POST only.
struct Rec {
    seen: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl Handler for Rec {
    fn route(&mut self, head: &Head) -> Result<(), Response> {
        if head.path != "/github" {
            return Err(Response::new(404, "not found"));
        }
        if head.method != "POST" {
            return Err(Response {
                allow: Some("POST"),
                ..Response::new(405, "method not allowed")
            });
        }
        Ok(())
    }
    fn handle(&mut self, request: Request) -> Response {
        self.seen.lock().unwrap().push(request.body);
        Response::new(200, "ok")
    }
}

struct Running {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    seen: Arc<Mutex<Vec<Vec<u8>>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn start(limits: Limits) -> Running {
    let listener = bind_loopback("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (s, sn) = (stop.clone(), seen.clone());
    let thread = std::thread::spawn(move || {
        serve(&listener, &limits, &s, &mut Rec { seen: sn });
    });
    Running {
        addr,
        stop,
        seen,
        thread: Some(thread),
    }
}

/// Send raw bytes, read the whole answer (the server closes the connection).
fn exchange(addr: SocketAddr, raw: &[u8]) -> String {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let _ = s.write_all(raw);
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    String::from_utf8_lossy(&out).into_owned()
}

fn status_of(answer: &str) -> u16 {
    answer
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

#[test]
fn a_post_to_the_path_reaches_the_handler_with_its_body() {
    let r = start(Limits::default());
    let a = exchange(
        r.addr,
        b"POST /github HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\nhello",
    );
    assert_eq!(status_of(&a), 200, "{a}");
    assert_eq!(r.seen.lock().unwrap().as_slice(), [b"hello".to_vec()]);
}

#[test]
fn the_wrong_path_is_404_and_the_wrong_method_is_405_and_neither_reaches_the_handler() {
    let r = start(Limits::default());
    let a = exchange(
        r.addr,
        b"POST /other HTTP/1.1\r\nContent-Length: 1\r\n\r\nx",
    );
    assert_eq!(status_of(&a), 404, "{a}");
    let a = exchange(
        r.addr,
        b"POST /github?x=1 HTTP/1.1\r\nContent-Length: 1\r\n\r\nx",
    );
    assert_eq!(status_of(&a), 404, "{a}");
    for method in ["GET", "PUT", "DELETE", "HEAD"] {
        let a = exchange(
            r.addr,
            format!("{method} /github HTTP/1.1\r\nContent-Length: 1\r\n\r\nx").as_bytes(),
        );
        assert_eq!(status_of(&a), 405, "{method}: {a}");
        assert!(a.contains("Allow: POST"), "{a}");
    }
    assert!(r.seen.lock().unwrap().is_empty());
}

#[test]
fn an_oversized_header_block_is_431() {
    let r = start(Limits::default());
    let mut raw = b"POST /github HTTP/1.1\r\nX-Pad: ".to_vec();
    raw.extend(std::iter::repeat_n(b'a', 40 * 1024));
    raw.extend_from_slice(b"\r\n\r\n");
    let a = exchange(r.addr, &raw);
    assert_eq!(status_of(&a), 431, "{a}");
    assert!(r.seen.lock().unwrap().is_empty());
}

#[test]
fn a_body_over_the_cap_is_413_before_it_is_read() {
    let limits = Limits {
        max_body: 1024,
        ..Limits::default()
    };
    let r = start(limits);
    // the declared length alone is enough; the client sends no body
    let a = exchange(
        r.addr,
        b"POST /github HTTP/1.1\r\nContent-Length: 1025\r\n\r\n",
    );
    assert_eq!(status_of(&a), 413, "{a}");
    // exactly the cap is fine
    let mut raw = b"POST /github HTTP/1.1\r\nContent-Length: 1024\r\n\r\n".to_vec();
    raw.extend(std::iter::repeat_n(b'x', 1024));
    assert_eq!(status_of(&exchange(r.addr, &raw)), 200);
    assert!(r.seen.lock().unwrap().iter().all(|b| b.len() == 1024));
}

#[test]
fn a_slow_client_is_cut_off_at_the_deadline() {
    let r = start(Limits {
        deadline: Duration::from_millis(300),
        ..Limits::default()
    });
    // headers never finished
    let started = Instant::now();
    let a = exchange(r.addr, b"POST /github HTTP/1.1\r\nHost: x\r\n");
    assert_eq!(status_of(&a), 408, "{a}");
    // body promised and never sent
    let a = exchange(
        r.addr,
        b"POST /github HTTP/1.1\r\nContent-Length: 10\r\n\r\nabc",
    );
    assert_eq!(status_of(&a), 408, "{a}");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(r.seen.lock().unwrap().is_empty());
}

#[test]
fn a_trickling_client_cannot_extend_the_deadline() {
    let r = start(Limits {
        deadline: Duration::from_millis(400),
        ..Limits::default()
    });
    let mut s = TcpStream::connect(r.addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let started = Instant::now();
    // one byte every 100 ms would take far longer than the deadline to finish the head
    for b in b"POST /github HTTP/1.1\r\nHost: x\r\nX-A: 1\r\n".iter() {
        if s.write_all(&[*b]).is_err() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
        if started.elapsed() > Duration::from_secs(3) {
            break;
        }
    }
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    assert_eq!(status_of(&String::from_utf8_lossy(&out)), 408);
    assert!(started.elapsed() < Duration::from_secs(4));
}

#[test]
fn a_post_without_content_length_is_411_and_chunked_is_501() {
    let r = start(Limits::default());
    let a = exchange(r.addr, b"POST /github HTTP/1.1\r\nHost: x\r\n\r\n");
    assert_eq!(status_of(&a), 411, "{a}");
    let a = exchange(
        r.addr,
        b"POST /github HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
    );
    assert_eq!(status_of(&a), 501, "{a}");
}

#[test]
fn garbage_is_400_and_a_silent_close_is_not_answered() {
    let r = start(Limits::default());
    assert_eq!(status_of(&exchange(r.addr, b"\x16\x03\x01\r\n\r\n")), 400);
    // connect and close at once: the listener carries on
    drop(TcpStream::connect(r.addr).unwrap());
    let a = exchange(
        r.addr,
        b"POST /github HTTP/1.1\r\nContent-Length: 1\r\n\r\nx",
    );
    assert_eq!(status_of(&a), 200);
}

/// A `serve_concurrent` on a loopback port whose handler panics on its first connection and answers `ok` after that;
/// the excess handler answers `busy`. Returns the address, the stop flag and the thread (its result is the server's
/// exit: `Err` when a handler's panic took the accept loop down with it).
fn panicking_server(max: usize) -> (SocketAddr, Arc<AtomicBool>, std::thread::JoinHandle<()>) {
    let listener = bind_loopback("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let first = AtomicBool::new(true);
    let t = std::thread::spawn(move || {
        serve_concurrent(
            &listener,
            &flag,
            max,
            Callbacks {
                handler: &|mut s: Stream| {
                    if first.swap(false, Ordering::SeqCst) {
                        panic!("a handler fault");
                    }
                    let _ = s.write_all(b"ok");
                },
                excess: &|mut s: Stream| {
                    let _ = s.write_all(b"busy");
                },
            },
        );
    });
    (addr, stop, t)
}

/// What the server says to a new connection, read under a deadline.
fn greeting(addr: SocketAddr) -> String {
    let mut c = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut out = String::new();
    let _ = c.read_to_string(&mut out);
    out
}

#[test]
fn a_handler_that_panics_gives_its_global_slot_back_and_the_server_goes_on() {
    // #1640: with a cap of one, one panic must not leave the server refusing everything
    let (addr, stop, t) = panicking_server(1);
    let _ = greeting(addr); // the connection whose handler panics
    let until = Instant::now() + Duration::from_secs(10);
    let mut said = String::new();
    while Instant::now() < until {
        said = greeting(addr);
        if said == "ok" {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    stop.store(true, Ordering::SeqCst);
    let joined = t.join();
    assert_eq!(said, "ok", "the slot of the panicked handler was kept");
    assert!(joined.is_ok(), "the handler's panic ended the accept loop");
}
