//! The real transport against a loopback server written in this file (plain sockets, 127.0.0.1 only, nothing leaves
//! the machine). TLS itself is rustls with the bundled web PKI roots and is not re-tested here; what is tested is that
//! the configuration never loosens it (see `the_agent_is_configured_as_strictly_as_documented`).

use super::*;
use crate::jev::fake::fixtures;
use crate::jev::retry::{Clock, NoJitter, RetryPolicy, Sleeper};
use crate::jev::transport::FilePart;
use crate::jev::{JevClient, JevConfig, JevError, Question};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub(super) const KEY: &str = "sk-live-SECRET-1234";

/// What a loopback server does with one connection.
enum Reply {
    /// Write these bytes, then close.
    Bytes(Vec<u8>),
    /// Accept and say nothing for a while (a stalled server).
    Hang,
}

pub(super) fn http(status: &str, headers: &[&str], body: &str) -> Reply {
    let mut s = format!("HTTP/1.1 {status}\r\nConnection: close\r\n");
    for h in headers {
        s += h;
        s += "\r\n";
    }
    if !headers
        .iter()
        .any(|h| h.to_ascii_lowercase().starts_with("transfer-encoding"))
    {
        s += &format!("Content-Length: {}\r\n", body.len());
    }
    s += "\r\n";
    s += body;
    Reply::Bytes(s.into_bytes())
}

pub(super) struct Server {
    port: u16,
    seen: Arc<Mutex<Vec<Vec<u8>>>>,
    count: Arc<AtomicUsize>,
}

impl Server {
    /// `script(n)` answers the n-th connection (from 0).
    fn start(script: impl Fn(usize) -> Reply + Send + 'static) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen: Arc<Mutex<Vec<Vec<u8>>>> = Arc::default();
        let count = Arc::new(AtomicUsize::new(0));
        let (s2, c2) = (seen.clone(), count.clone());
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut conn) = conn else { return };
                let n = c2.fetch_add(1, Ordering::SeqCst);
                let raw = read_request(&mut conn);
                s2.lock().unwrap().push(raw);
                match script(n) {
                    Reply::Bytes(b) => {
                        let _ = conn.write_all(&b);
                        let _ = conn.flush();
                    }
                    Reply::Hang => std::thread::sleep(Duration::from_secs(5)),
                }
            }
        });
        Server { port, seen, count }
    }

    fn connections(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }

    fn request(&self, n: usize) -> Vec<u8> {
        self.seen.lock().unwrap()[n].clone()
    }

    fn head(&self, n: usize) -> String {
        let raw = self.request(n);
        let end = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .unwrap_or(raw.len());
        String::from_utf8_lossy(&raw[..end]).to_string()
    }
}

pub(super) fn read_request(conn: &mut TcpStream) -> Vec<u8> {
    conn.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut buf = vec![];
    let mut chunk = [0u8; 4096];
    while let Ok(n) = conn.read(&mut chunk) {
        buf.extend(&chunk[..n]);
        if n == 0 || request_complete(&buf) {
            break;
        }
    }
    buf
}

/// True when the head is in and as many body bytes as its content-length says.
fn request_complete(buf: &[u8]) -> bool {
    let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
        return false;
    };
    let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
    let want = head
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    buf.len() >= end + 4 + want
}

pub(super) fn opts() -> HttpOptions {
    HttpOptions {
        connect_timeout: Duration::from_secs(2),
        total_timeout: Duration::from_secs(3),
        max_reply_bytes: 64 * 1024,
    }
}

pub(super) fn loopback(server: &Server) -> HttpTransport {
    HttpTransport::loopback_for_tests(server.port, false, opts())
}

pub(super) fn key() -> ApiKey {
    ApiKey::new(KEY.to_string())
}

pub(super) fn status_of(e: JevTransportError) -> (u16, String, RetryHeaders) {
    match e {
        JevTransportError::Status {
            status,
            body,
            headers,
        } => (status, body, headers),
        other => panic!("expected a status, got {other:?}"),
    }
}

mod faults;
mod key;
mod requests;
mod target;
