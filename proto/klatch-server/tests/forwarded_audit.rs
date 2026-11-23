//! Forwarded identity headers are ignored and counted (#110, epic #13; the owner's decision of 2026-11-23: klatch never
//! trusts a proxy's word about who a caller is). A request that carries one or more of the fixed list of such headers
//! (`src/forwarded.rs`) gives no identity, and bumps an in-memory counter by header name. The counter holds names only,
//! never a value, and its summary is the counts-only line the server writes to stderr when it stops. That a header
//! changes no principal is tested in `http_wire.rs`; this file tests the count.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

static N: AtomicU32 = AtomicU32::new(0);

/// A short directory under /tmp with a store home in it.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Dir {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = PathBuf::from(format!("/tmp/kf{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["", "h"] {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(dir.join(sub))
                .unwrap();
        }
        Dir(dir)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Server {
    child: Child,
    addr: SocketAddr,
    log: PathBuf,
}

impl Server {
    fn started(dir: &Dir) -> Server {
        let home = dir.0.join("h");
        let log = dir.0.join("err.log");
        let child = Command::new(env!("CARGO_BIN_EXE_klatch-server"))
            .args(["http", "--listen", "127.0.0.1:0"])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &home)
            .env("KLATCH_HOME", &home)
            .env("KLATCH_KEY_DIR", home.join("keys"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log)
                    .unwrap(),
            ))
            .spawn()
            .expect("start klatch-server");
        let mut server = Server {
            child,
            addr: "127.0.0.1:0".parse().unwrap(),
            log,
        };
        let until = Instant::now() + Duration::from_secs(60);
        while Instant::now() < until {
            let text = std::fs::read_to_string(&server.log).unwrap_or_default();
            if let Some(rest) = text.split("http listening on ").nth(1)
                && rest.contains('\n')
            {
                server.addr = rest.split_whitespace().next().unwrap().parse().unwrap();
                return server;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("the server never listened");
    }

    /// Stop it with SIGTERM and return everything it wrote to stderr.
    fn stop(mut self) -> String {
        unsafe {
            libc::kill(self.child.id() as libc::pid_t, libc::SIGTERM);
        }
        let until = Instant::now() + Duration::from_secs(20);
        while self.child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < until, "the server did not stop");
            std::thread::sleep(Duration::from_millis(20));
        }
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

/// Send one request and return its status code.
fn status_of(addr: SocketAddr, (method, path): (&str, &str), headers: &[(&str, &str)]) -> u16 {
    let mut s = TcpStream::connect(addr).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\n");
    for (n, v) in headers {
        head.push_str(&format!("{n}: {v}\r\n"));
    }
    if method == "POST" {
        head.push_str("Content-Length: 2\r\n");
    }
    head.push_str("\r\n");
    let _ = s.write_all(head.as_bytes());
    if method == "POST" {
        let _ = s.write_all(b"{}");
    }
    let mut r = BufReader::new(s);
    let mut line = String::new();
    r.read_line(&mut line).expect("a status line");
    let status = line.split_whitespace().nth(1).unwrap().parse().unwrap();
    let mut rest = String::new();
    let _ = r.read_to_string(&mut rest);
    status
}

/// The one summary line of the stopped server's log.
fn summary(log: &str) -> &str {
    log.lines()
        .find(|l| l.contains("forwarded identity headers ignored"))
        .unwrap_or_else(|| panic!("no summary line in:\n{log}"))
}

/// Every name of the fixed list is one count; a request is counted once however many it carries or repeats.
#[test]
fn the_counter_counts_requests_by_header_name_and_nothing_else() {
    let dir = Dir::new();
    let s = Server::started(&dir);
    // no forwarded header: not counted, whatever the route
    assert_eq!(status_of(s.addr, ("GET", "/v1/stream"), &[]), 401);
    assert_eq!(status_of(s.addr, ("GET", "/nowhere"), &[]), 404);
    // one request, one header
    assert_eq!(
        status_of(
            s.addr,
            ("GET", "/v1/stream"),
            &[("X-Forwarded-For", "10.0.0.7")]
        ),
        401
    );
    // one request, three headers (one repeated): one request, each name once
    let many = [
        ("X-Forwarded-For", "10.9.9.9"),
        ("x-forwarded-for", "10.8.8.8"),
        ("Forwarded", "for=10.7.7.7"),
        ("X-Remote-User", "w-1"),
    ];
    assert_eq!(status_of(s.addr, ("POST", "/v1/frames"), &many), 401);
    // a prefix family is one name, counted once per request; an unrouted request is counted too
    let certs = [
        ("X-SSL-Client-S-DN", "CN=w-1"),
        ("X-SSL-Client-Verify", "SUCCESS"),
    ];
    assert_eq!(status_of(s.addr, ("POST", "/mcp"), &certs), 401);
    assert_eq!(
        status_of(s.addr, ("GET", "/nowhere"), &[("X-Real-IP", "10.6.6.6")]),
        404
    );
    let log = s.stop();
    let line = summary(&log);
    assert!(line.contains("requests=4"), "{line}");
    assert!(line.contains("x-forwarded-for=2"), "{line}");
    assert!(line.contains("forwarded=1"), "{line}");
    assert!(line.contains("x-remote-user=1"), "{line}");
    assert!(line.contains("x-ssl-client-*=1"), "{line}");
    assert!(line.contains("x-real-ip=1"), "{line}");
    assert!(
        !line.contains("x-client-cert"),
        "a name never sent is not listed: {line}"
    );
}

/// Names only: no address, user or certificate text from a header reaches the log.
#[test]
fn no_header_value_reaches_the_log() {
    let dir = Dir::new();
    let s = Server::started(&dir);
    let sent = [
        ("X-Forwarded-For", "203.0.113.77"),
        ("X-Forwarded-User", "mallory-the-forwarded"),
        (
            "X-Forwarded-Client-Cert",
            "Hash=deadbeefcafe;Subject=CN=evil",
        ),
    ];
    assert_eq!(status_of(s.addr, ("GET", "/v1/stream"), &sent), 401);
    let log = s.stop();
    for secret in ["203.0.113.77", "mallory", "deadbeefcafe", "evil"] {
        assert!(!log.contains(secret), "{secret} leaked into:\n{log}");
    }
    assert!(summary(&log).contains("requests=1"), "{log}");
}

/// With none sent the summary still appears, at zero, so an operator can tell "none" from "not counted".
#[test]
fn a_server_that_saw_none_reports_zero() {
    let dir = Dir::new();
    let s = Server::started(&dir);
    assert_eq!(status_of(s.addr, ("GET", "/v1/stream"), &[]), 401);
    let log = s.stop();
    assert!(summary(&log).contains("requests=0"), "{log}");
}

/// The WebSocket route (#112) is behind the same counter: an upgrade request with forwarded headers is counted by name,
/// and no value reaches the log.
#[test]
fn an_upgrade_request_to_the_websocket_route_is_counted_by_header_name_and_never_by_value() {
    let dir = Dir::new();
    let s = Server::started(&dir);
    let headers = [
        ("Upgrade", "websocket"),
        ("Connection", "Upgrade"),
        ("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ=="),
        ("Sec-WebSocket-Version", "13"),
        ("X-Forwarded-For", "10.4.4.4"),
        ("X-Remote-User", "mallory"),
    ];
    assert_eq!(status_of(s.addr, ("GET", "/v1/ws"), &headers), 401);
    let log = s.stop();
    let line = summary(&log);
    assert!(line.contains("requests=1"), "{line}");
    assert!(
        line.contains("x-forwarded-for=1") && line.contains("x-remote-user=1"),
        "{line}"
    );
    assert!(
        !log.contains("10.4.4.4") && !log.contains("mallory"),
        "{log}"
    );
}
