//! The one place this crate listens on a socket (#1058, epic #1056), and the one place the scan test
//! (`tests/no_network.rs`) lets a `TcpListener` appear. It is a small, single-purpose HTTP/1.1 listener for loopback
//! only, built to be reused by phases 3 and 4 (the klatch wire over HTTP and MCP over Streamable HTTP):
//!
//! - [`bind_loopback`] binds or REFUSES: the address must be exactly `127.0.0.1` or `::1` with a port (no host name, no
//!   resolution, no `0.0.0.0`, no other `127.x` address, no IPv4-mapped form). A refusal is an error naming the
//!   zero-trust prerequisites, never a warning.
//! - [`serve`] accepts one connection at a time, reads one request under hard limits ([`Limits`]: head bytes, body
//!   bytes, one total deadline per request), asks a [`Handler`] first about the head (route, method) and only then
//!   reads the body, answers, and closes the connection (`Connection: close`; no keep-alive, so an idle client cannot
//!   hold the listener).
//!
//! Nothing here connects outward and nothing starts a program. A request is parsed only as far as the head and a
//! `Content-Length` body; `Transfer-Encoding` is refused (501), so there is no chunked parser to get wrong.

use anyhow::{Result, bail};
use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Why a non-loopback bind is refused, in the words of the error.
pub const ZERO_TRUST_PREREQUISITES: &str = "a listener beyond this machine needs encrypted and mutually authenticated transports (#157) and per-request authentication, authorization and short-lived credentials (#161) first";

/// The only addresses a listener may bind: `127.0.0.1` and `::1`.
fn is_allowed_ip(ip: &IpAddr) -> bool {
    *ip == IpAddr::V4(Ipv4Addr::LOCALHOST) || *ip == IpAddr::V6(Ipv6Addr::LOCALHOST)
}

/// Parse an address and refuse it unless it is `127.0.0.1:PORT` or `[::1]:PORT`. The text is parsed, never resolved, so
/// a host name (even `localhost`) is refused.
pub fn check_loopback(addr: &str) -> Result<SocketAddr> {
    let Ok(parsed) = addr.parse::<SocketAddr>() else {
        bail!(
            "refusing to listen on {:?}: the address must be 127.0.0.1:PORT or [::1]:PORT (an IP address, not a host name); {ZERO_TRUST_PREREQUISITES}",
            klatch::domain::flat(addr)
        );
    };
    if !is_allowed_ip(&parsed.ip()) {
        bail!(
            "refusing to listen on {parsed}: only 127.0.0.1 and ::1 are allowed; {ZERO_TRUST_PREREQUISITES}"
        );
    }
    Ok(parsed)
}

/// The listener type, named here so that no other file needs the socket types (the scan test allows them in this one).
pub type Listener = TcpListener;

/// Check the address and bind it. A refusal happens before any socket exists.
pub fn bind_loopback(addr: &str) -> Result<TcpListener> {
    let parsed = check_loopback(addr)?;
    let listener = TcpListener::bind(parsed)
        .map_err(|e| anyhow::anyhow!("cannot listen on {parsed}: {}", e.kind()))?;
    // the bound address is loopback by construction; check what the system says anyway
    if !is_allowed_ip(&listener.local_addr()?.ip()) {
        bail!("refusing to listen: the bound address is not loopback");
    }
    Ok(listener)
}

/// The bounds on one request.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// The request line and headers, up to and including the blank line.
    pub max_head: usize,
    /// The body (`Content-Length`).
    pub max_body: usize,
    /// The whole request, from the first byte read: a slow client is cut off at this deadline.
    pub deadline: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_head: 16 * 1024,
            max_body: 1024 * 1024,
            deadline: Duration::from_secs(5),
        }
    }
}

/// The request line and headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub method: String,
    /// The request target as sent (path and query).
    pub path: String,
    /// Header names lower-cased, values trimmed.
    pub headers: Vec<(String, String)>,
    pub content_length: Option<usize>,
}

impl Head {
    /// The value of a header (case-insensitive name), when it was sent.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A whole request: the head and the body (at most `Limits::max_body` bytes).
#[derive(Debug)]
pub struct Request {
    pub head: Head,
    pub body: Vec<u8>,
}

/// What the listener answers. The body is a short fixed text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: &'static str,
    /// The `Allow` header of a 405.
    pub allow: Option<&'static str>,
}

impl Response {
    pub fn new(status: u16, body: &'static str) -> Self {
        Response {
            status,
            body,
            allow: None,
        }
    }

    fn reason(&self) -> &'static str {
        match self.status {
            200 => "OK",
            202 => "Accepted",
            400 => "Bad Request",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            408 => "Request Timeout",
            411 => "Length Required",
            413 => "Content Too Large",
            429 => "Too Many Requests",
            431 => "Request Header Fields Too Large",
            501 => "Not Implemented",
            503 => "Service Unavailable",
            _ => "Status",
        }
    }

    pub fn bytes(&self) -> Vec<u8> {
        let mut head = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n",
            self.status,
            self.reason(),
            self.body.len()
        );
        if let Some(allow) = self.allow {
            head.push_str(&format!("Allow: {allow}\r\n"));
        }
        head.push_str("\r\n");
        let mut out = head.into_bytes();
        out.extend_from_slice(self.body.as_bytes());
        out
    }
}

/// Why a request was not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// The deadline passed (a slow or silent client).
    Timeout,
    /// More than `max_head` bytes without the end of the headers.
    HeadTooLarge,
    /// A `Content-Length` over `max_body`.
    BodyTooLarge,
    /// Not an HTTP request this listener understands.
    Malformed(&'static str),
    /// A body-carrying method without a `Content-Length`.
    LengthRequired,
    /// `Transfer-Encoding` (chunked bodies are not read).
    Unsupported,
    /// The client went away mid-request: nothing to answer.
    Closed,
}

impl Fault {
    /// The answer, `None` when the client is gone.
    pub fn response(self) -> Option<Response> {
        Some(match self {
            Fault::Timeout => Response::new(408, "request timeout"),
            Fault::HeadTooLarge => Response::new(431, "headers too large"),
            Fault::BodyTooLarge => Response::new(413, "body too large"),
            Fault::Malformed(why) => Response::new(400, why),
            Fault::LengthRequired => Response::new(411, "content-length required"),
            Fault::Unsupported => Response::new(501, "transfer-encoding is not supported"),
            Fault::Closed => return None,
        })
    }
}

fn is_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// A method is a short token.
fn is_method(method: &str) -> bool {
    is_token(method) && method.len() <= 16
}

/// The request line: method, target and protocol version, exactly three parts.
fn parse_request_line(line: &str) -> Result<(&str, &str), Fault> {
    let mut parts = line.split(' ');
    let (Some(method), Some(path), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(Fault::Malformed("bad request line"));
    };
    if !is_method(method) || !path.starts_with('/') {
        return Err(Fault::Malformed("bad request line"));
    }
    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        return Err(Fault::Malformed("unsupported protocol version"));
    }
    Ok((method, path))
}

/// A `Content-Length` value: digits only, at most twelve of them.
fn parse_content_length(value: &str) -> Result<usize, Fault> {
    let bad = Fault::Malformed("bad content-length");
    let digits = !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit());
    if !digits || value.len() > 12 {
        return Err(bad);
    }
    value.parse::<usize>().map_err(|_| bad)
}

/// Parse the request line and headers (the bytes before the blank line).
pub fn parse_head(raw: &[u8]) -> Result<Head, Fault> {
    let text = std::str::from_utf8(raw).map_err(|_| Fault::Malformed("headers are not text"))?;
    let mut lines = text.split("\r\n");
    let (method, path) = parse_request_line(lines.next().unwrap_or(""))?;
    let mut headers = Vec::new();
    let mut content_length = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(Fault::Malformed("bad header line"));
        };
        if !is_token(name) {
            return Err(Fault::Malformed("bad header name"));
        }
        let name = name.to_ascii_lowercase();
        let value = value.trim().to_string();
        if name == "transfer-encoding" {
            return Err(Fault::Unsupported);
        }
        if name == "content-length" {
            if content_length.is_some() {
                return Err(Fault::Malformed("duplicate content-length"));
            }
            content_length = Some(parse_content_length(&value)?);
        }
        headers.push((name, value));
    }
    Ok(Head {
        method: method.to_string(),
        path: path.to_string(),
        headers,
        content_length,
    })
}

/// Read some bytes before `deadline`.
fn read_some(stream: &mut TcpStream, buf: &mut [u8], deadline: Instant) -> Result<usize, Fault> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(Fault::Timeout);
    }
    stream
        .set_read_timeout(Some(left))
        .map_err(|_| Fault::Closed)?;
    loop {
        match stream.read(buf) {
            Ok(0) => return Err(Fault::Closed),
            Ok(n) => return Ok(n),
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                return Err(Fault::Timeout);
            }
            Err(_) => return Err(Fault::Closed),
        }
    }
}

/// Read the request line and headers: the head and any body bytes that arrived with it. At most `limits.max_head` bytes
/// of head are kept; a client that never ends the headers is stopped there or at the deadline.
pub fn read_head(
    stream: &mut TcpStream,
    limits: &Limits,
    deadline: Instant,
) -> Result<(Head, Vec<u8>), Fault> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 2048];
    loop {
        if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            if at > limits.max_head {
                return Err(Fault::HeadTooLarge);
            }
            let head = parse_head(&buf[..at])?;
            return Ok((head, buf[at + 4..].to_vec()));
        }
        if buf.len() > limits.max_head {
            return Err(Fault::HeadTooLarge);
        }
        let n = read_some(stream, &mut chunk, deadline)?;
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// The bounds one request is read under: its limits and the moment its deadline passes.
#[derive(Clone, Copy)]
pub struct Budget<'a> {
    pub limits: &'a Limits,
    pub deadline: Instant,
}

/// Read the `Content-Length` body (`first` is what arrived with the head).
pub fn read_body(
    stream: &mut TcpStream,
    want: usize,
    mut first: Vec<u8>,
    budget: Budget,
) -> Result<Vec<u8>, Fault> {
    if want > budget.limits.max_body {
        return Err(Fault::BodyTooLarge);
    }
    first.truncate(want);
    let mut chunk = [0u8; 8192];
    while first.len() < want {
        let room = (want - first.len()).min(chunk.len());
        let n = read_some(stream, &mut chunk[..room], budget.deadline)?;
        first.extend_from_slice(&chunk[..n]);
    }
    Ok(first)
}

/// A request whose head has been read and whose body has not: what the routes that read a body share.
pub struct Pending<'a> {
    pub stream: &'a mut TcpStream,
    pub head: Head,
    /// Body bytes that arrived with the head.
    pub first: Vec<u8>,
    pub budget: Budget<'a>,
}

impl Pending<'_> {
    /// Whether the head declares a body over the limit (refused before anything is read).
    pub fn declares_too_large(&self) -> bool {
        self.head
            .content_length
            .is_some_and(|n| n > self.budget.limits.max_body)
    }

    /// The body the head declares, or the answer that refuses it: `Content-Length` is required, and a client that is
    /// gone gets a status 0 answer (nothing is sent).
    pub fn body(&mut self) -> Result<Vec<u8>, Response> {
        let Some(want) = self.head.content_length else {
            return Err(Fault::LengthRequired.response().unwrap());
        };
        let first = std::mem::take(&mut self.first);
        read_body(self.stream, want, first, self.budget)
            .map_err(|f| f.response().unwrap_or_else(|| Response::new(0, "")))
    }
}

/// What the listener asks of its user.
pub trait Handler {
    /// Look at the head before the body is read: `Err` answers at once (404 for the path, 405 for the method).
    fn route(&mut self, head: &Head) -> Result<(), Response>;
    /// Answer a whole request that `route` accepted.
    fn handle(&mut self, request: Request) -> Response;
}

/// Read one request from a connection and answer it.
fn serve_one(stream: &mut TcpStream, limits: &Limits, handler: &mut dyn Handler) {
    let deadline = Instant::now() + limits.deadline;
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let response = (|| -> Result<Response, Fault> {
        let (head, first) = read_head(stream, limits, deadline)?;
        if let Err(r) = handler.route(&head) {
            return Ok(r);
        }
        let body = match head.content_length {
            Some(n) => read_body(stream, n, first, Budget { limits, deadline })?,
            None if head.method == "POST" || head.method == "PUT" => {
                return Err(Fault::LengthRequired);
            }
            None => Vec::new(),
        };
        Ok(handler.handle(Request { head, body }))
    })()
    .unwrap_or_else(|f| f.response().unwrap_or_else(|| Response::new(0, "")));
    if response.status != 0 {
        let _ = stream.write_all(&response.bytes());
        let _ = stream.flush();
    }
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

/// A connection as [`serve_concurrent`] hands it to its handler. The alias is how the other modules name the socket
/// without naming its type (the scan test lets only this file do that).
pub type Stream = TcpStream;

/// What [`serve_concurrent`] calls: `handler` serves a connection, `excess` answers one over the cap.
pub struct Callbacks<'a, H, X> {
    pub handler: &'a H,
    pub excess: &'a X,
}

/// The open connections, by number, so that stopping can shut them all down.
type OpenConnections = Mutex<HashMap<u64, TcpStream>>;

/// Make an accepted socket blocking (it may inherit the listener's non-blocking mode) and clone a handle to keep.
fn blocking_handle(stream: &TcpStream) -> Option<TcpStream> {
    stream.set_nonblocking(false).ok()?;
    stream.try_clone().ok()
}

/// Take a slot for a connection unless `max_connections` are open: its number, or `None` when the cap is reached.
fn claim_slot(
    open: &OpenConnections,
    max_connections: usize,
    next: &mut u64,
    handle: TcpStream,
) -> Option<u64> {
    let mut open = open.lock().unwrap_or_else(|e| e.into_inner());
    if open.len() >= max_connections {
        return None;
    }
    *next += 1;
    open.insert(*next, handle);
    Some(*next)
}

/// Accept connections until `stop` is set and serve each on a thread of its own (phase 3, #1059: a server-sent-events
/// stream stays open, so the one-at-a-time loop of [`serve`] cannot carry it). At most `max_connections` are open at
/// once: the excess is given to `excess` (which should answer in a moment and close) and gets no thread. On `stop` every
/// open connection is shut down, so a handler blocked on its socket returns, and the threads are joined before this
/// returns. The listener is polled, as in [`serve`]. Nothing here reads a request: the handler owns the connection.
pub fn serve_concurrent<H, X>(
    listener: &TcpListener,
    stop: &AtomicBool,
    max_connections: usize,
    callbacks: Callbacks<H, X>,
) where
    H: Fn(Stream) + Sync,
    X: Fn(Stream) + Sync,
{
    if listener.set_nonblocking(true).is_err() {
        return;
    }
    let open: OpenConnections = Mutex::default();
    std::thread::scope(|scope| {
        let mut next = 0u64;
        while !stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let Some(handle) = blocking_handle(&stream) else {
                        continue;
                    };
                    let Some(id) = claim_slot(&open, max_connections, &mut next, handle) else {
                        (callbacks.excess)(stream);
                        continue;
                    };
                    let (open, handler) = (&open, callbacks.handler);
                    scope.spawn(move || {
                        handler(stream);
                        open.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                    });
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        for s in open.lock().unwrap_or_else(|e| e.into_inner()).values() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    });
}

/// Accept connections one at a time until `stop` is set. The listener is polled (a short pause when nobody is
/// connecting), so `stop` is noticed within about 20 ms.
pub fn serve(
    listener: &TcpListener,
    limits: &Limits,
    stop: &AtomicBool,
    handler: &mut dyn Handler,
) {
    if listener.set_nonblocking(true).is_err() {
        return;
    }
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                // an accepted socket may inherit the listener's non-blocking mode
                if stream.set_nonblocking(false).is_ok() {
                    serve_one(&mut stream, limits, handler);
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}
