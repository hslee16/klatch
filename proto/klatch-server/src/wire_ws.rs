//! `GET /v1/ws` (#112, epic #13): the klatch wire protocol over one WebSocket connection, on the listeners that already
//! exist (`klatch-server http` on loopback, `klatch-server https` behind mutual TLS). It is not a new protocol and not a new
//! listener: the connection is upgraded inside `wire_http::serve_connection`, runs the session loop every transport runs
//! (`Session` over the one shared gateway), and carries the same frame lines as `GET /v1/stream` and `POST /v1/frames`, one
//! frame per text message each way. Requests, events, acknowledgements and resume from `seq` (a `subscribe` with `from_seq`)
//! mean here what they mean there. `ws://` is the loopback listener, `wss://` is the TLS listener; there is no other way
//! to a WebSocket, so a WebSocket off loopback is always TLS with a client certificate (#108).
//!
//! **Who is calling.** The upgrade needs `Authorization: Bearer <token>` and `X-Klatch-Member: <member>`, a worker credential
//! checked live by `wire_http::authenticate`, the check `GET /v1/stream` makes. Every refusal is the one `401 unauthorized`
//! before anything else is read or said; a client certificate gates the port and names nobody; forwarded identity headers
//! are never read. The credential is checked once, at the upgrade, and then by the gateway on every request the session
//! runs (a revoked or expired credential stops working at once, as on every transport). Message text is data.
//!
//! **Origin is not an identity.** A browser page cannot set `Authorization` on a WebSocket and has no way to reach a
//! loopback or TLS port with a credential, but a page on another site can still open a connection to `127.0.0.1` (and a
//! DNS rebinding page can reach it under its own name), so a request that carries an `Origin` header is refused with `403`
//! (after the credential: a stranger gets the same `401` as anywhere) unless the operator named that exact origin with
//! `--ws-allow-origin` (off by default, repeatable, no wildcard). Clients that are not browsers send none.
//!
//! **Frames.** Client frames must be masked (RFC 6455; an unmasked one is a protocol error, 1002). A message is at most 1 MiB
//! (`Limits::max_body`, the HTTP body limit, and the session's line limit), whole or in fragments: a frame that declares
//! more is refused at its header (1009) and fragments are counted as they arrive, never held past the limit. Only text
//! messages are frames; binary is refused (1003) and text that is not UTF-8 is 1007. Control frames are at most 125 bytes
//! and never fragmented. No extension (no compression) and no subprotocol is offered. A text message holds the frame lines
//! of a POST body: one line is one frame, and a message with several lines is several frames.
//!
//! **Every wait is bounded.** The head is read under the HTTP deadline (5 seconds). Afterwards the server pings every 20
//! seconds and closes a connection that sent no complete frame (a pong counts) for 60 seconds (1001); a write that cannot
//! finish within the session's stall time ends the session; a close is waited on for 2 seconds and then the socket is shut.
//! The server's reader owns reads and polls with a 100 ms tick, the session's writer owns writes, and both take the one
//! lock around the protocol state for the length of one call, never across a wait. The connection holds one slot of the
//! listener's `--max-connections` for its whole life, and a guard shuts the socket on every exit path.
//!
//! This module names no socket type: the connection is a `loopback::Stream`, and TLS is the pump's business (`tls_bridge`).

use crate::inbox::Inbox;
use crate::loopback::{Head, Limits, Response, Stream};
use crate::wire_http::{Shared, authenticate, reply_bytes, unauthorized};
use crate::wire_ws_caps::Settings;
use klatch::domain::Principal;
use klatch::protocol::MAX_LINE_BYTES;
use klatch::session::{Closer, Session};
use std::io::{self, BufRead, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tungstenite::error::ProtocolError;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::{CloseFrame, Role, WebSocket, WebSocketConfig};
use tungstenite::{Error as WsError, Message};

pub const PATH: &str = "/v1/ws";

/// Message lines a connection may have waiting to be read by its session loop (the HTTP wire's inbox size). A client that
/// sends faster than the session reads is not read from until the session catches up: its data waits in TCP.
const INBOX: usize = 16;
/// How long the reader and the writer wait for the socket at a time, and so how soon either notices that the other ended.
const TICK_MS: libc::c_int = 100;
/// RFC 6455 section 1.3: the value hashed with the client's key.
const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

// ---------------------------------------------------------------------------------------------------------------
// the upgrade

/// The values of every header named `name` (lower-case), in order.
fn values<'a>(head: &'a Head, name: &str) -> Vec<&'a str> {
    head.headers
        .iter()
        .filter(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
        .collect()
}

/// A page on another origin is refused unless the operator named it. Every `Origin` header counts, even an empty one, and
/// there must be exactly one: the answer is the whole refusal.
fn check_origin(head: &Head, allowed: &[String]) -> Result<(), Vec<u8>> {
    let origins = values(head, "origin");
    let fine =
        origins.is_empty() || (origins.len() == 1 && allowed.iter().any(|a| a == origins[0]));
    fine.then_some(())
        .ok_or_else(|| Response::new(403, "origin not allowed").bytes())
}

/// Standard base64 with padding.
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().fold(0u32, |n, b| n << 8 | *b as u32) << (8 * (3 - chunk.len()));
        for i in 0..4 {
            out.push(if i <= chunk.len() {
                ABC[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

/// The `Sec-WebSocket-Accept` value for a key: base64 of the SHA-1 of the key and the GUID (RFC 6455 section 4.2.2). The
/// digest proves only that the server understood the handshake; it is not a secret and protects nothing.
fn accept_key(key: &str) -> String {
    let digest = ring::digest::digest(
        &ring::digest::SHA1_FOR_LEGACY_USE_ONLY,
        format!("{key}{GUID}").as_bytes(),
    );
    base64(digest.as_ref())
}

/// A key is 16 random bytes in base64: 24 characters, the last two `=`.
fn is_key(key: &str) -> bool {
    key.len() == 24
        && key.ends_with("==")
        && key[..22]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
}

fn refusal(status: u16, text: &'static str) -> Vec<u8> {
    Response::new(status, text).bytes()
}

/// Exactly one `Upgrade: websocket` and an `upgrade` token among the `Connection` headers.
fn is_upgrade(head: &Head) -> bool {
    let upgrade = values(head, "upgrade");
    let connection = values(head, "connection");
    let mut tokens = connection.iter().flat_map(|v| v.split(',')).map(str::trim);
    let named = upgrade.len() == 1 && upgrade[0].eq_ignore_ascii_case("websocket");
    named && tokens.any(|t| t.eq_ignore_ascii_case("upgrade"))
}

/// The refusal of a version other than 13 (`426`, naming the version taken) or of a missing one, or `None` for 13.
fn version_refusal(head: &Head) -> Option<Vec<u8>> {
    match values(head, "sec-websocket-version").as_slice() {
        ["13"] => None,
        [] => Some(refusal(400, "sec-websocket-version is required")),
        _ => {
            let body = "unsupported websocket version";
            let text = format!(
                "HTTP/1.1 426 Upgrade Required\r\nSec-WebSocket-Version: 13\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            Some(text.into_bytes())
        }
    }
}

/// Whether the request is a WebSocket upgrade this server takes: the accept value to answer with, or the answer that
/// refuses it. HTTP/1.1, exactly one of each header, version 13, a well-formed key, nothing sent ahead of the answer.
fn check_upgrade(head: &Head, first: &[u8]) -> Result<String, Vec<u8>> {
    if head.http10 {
        return Err(refusal(400, "a websocket upgrade needs http/1.1"));
    }
    if !is_upgrade(head) {
        return Err(refusal(400, "a websocket upgrade is required"));
    }
    if let Some(answer) = version_refusal(head) {
        return Err(answer);
    }
    let key = match values(head, "sec-websocket-key").as_slice() {
        [key] if is_key(key) => *key,
        _ => return Err(refusal(400, "sec-websocket-key is not valid")),
    };
    if !first.is_empty() || head.content_length.is_some_and(|n| n != 0) {
        return Err(refusal(400, "nothing may be sent before the answer"));
    }
    Ok(accept_key(key))
}

/// `GET /v1/ws`: the credential first, then the origin, then the upgrade, then the member's place among her open
/// connections; each refusal is final. The refusal at her cap is the answer of a full server (`503 busy`): it does not say
/// who holds the places.
pub(crate) fn serve(mut stream: Stream, head: &Head, first: &[u8], shared: &Shared) {
    let Some(who) = authenticate(&shared.backend, head) else {
        return reply_bytes(&mut stream, &unauthorized().bytes());
    };
    let accept =
        match check_origin(head, &shared.ws.origins).and_then(|()| check_upgrade(head, first)) {
            Ok(accept) => accept,
            Err(answer) => return reply_bytes(&mut stream, &answer),
        };
    let Some(slot) = shared.ws_open.claim(&who.member, shared.ws.per_member) else {
        return reply_bytes(&mut stream, &Response::new(503, "busy").bytes());
    };
    connection(stream, &who, shared, &accept);
    drop(slot);
}

// ---------------------------------------------------------------------------------------------------------------
// the connection

type Socket = Arc<Mutex<WebSocket<Stream>>>;

/// The protocol state, for the length of one call.
fn lock(ws: &Mutex<WebSocket<Stream>>) -> MutexGuard<'_, WebSocket<Stream>> {
    ws.lock().unwrap_or_else(|e| e.into_inner())
}

fn would_block(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
    )
}

/// Wait up to a tick for the descriptor (`POLLIN` or `POLLOUT`); the caller looks at the socket itself afterwards.
fn wait_fd(fd: RawFd, events: libc::c_short) {
    let mut p = libc::pollfd {
        fd,
        events,
        revents: 0,
    };
    // SAFETY: p is one valid pollfd for the duration of the call.
    unsafe {
        libc::poll(&mut p, 1, TICK_MS);
    }
}

/// The frame layer's own limits: a message is at most the HTTP body limit, frame and message alike; nothing unmasked is
/// accepted; an unsent backlog is bounded.
fn config() -> WebSocketConfig {
    let max = Limits::default().max_body;
    WebSocketConfig::default()
        .max_message_size(Some(max))
        .max_frame_size(Some(max))
        .max_write_buffer_size(4 * max)
        .accept_unmasked_frames(false)
}

/// Shuts the socket when it fires or is dropped, so every way out of a connection (an error, a panic, the end of the
/// session) ends in a closed socket and no thread stays waiting on it.
struct Teardown(Stream);

impl Teardown {
    fn fire(&self) {
        let _ = self.0.shutdown(std::net::Shutdown::Both);
    }
}

impl Drop for Teardown {
    fn drop(&mut self) {
        self.fire();
    }
}

/// Sets its flag when dropped: the reader is done, however it ended.
struct Done<'a>(&'a AtomicBool);

impl Drop for Done<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// What the reader does next.
#[derive(PartialEq)]
enum Step {
    Go,
    End,
}

/// The reading end: it owns every read, forwards text messages to the session, answers nothing itself (the protocol layer
/// queues pongs and the close reply) and enforces the idle time, the ping and the close wait.
struct Reader<'a> {
    ws: Socket,
    fd: RawFd,
    tx: Option<SyncSender<Vec<u8>>>,
    settings: &'a Settings,
    closed: Arc<AtomicBool>,
    last_rx: Instant,
    last_ping: Instant,
    /// When a close began (either side); the connection ends `close_wait` after.
    closing: Option<Instant>,
    /// When the connection was upgraded: it ends at the maximum age, however lively.
    born: Instant,
}

impl Reader<'_> {
    fn run(mut self) {
        while self.turn() == Step::Go {}
    }

    /// One turn: a close the session asked for or the age limit, then one read and what it brings.
    fn turn(&mut self) -> Step {
        if self.closed.load(Ordering::SeqCst) {
            self.begin_close();
        } else if self.closing.is_none() && self.born.elapsed() >= self.settings.max_age {
            self.refuse(
                CloseCode::Away,
                "connection age limit, reconnect and resume",
            );
        }
        let read = lock(&self.ws).read();
        match read {
            Ok(message) => self.message(message),
            Err(WsError::Io(e)) if would_block(&e) => {
                wait_fd(self.fd, libc::POLLIN);
                self.tick()
            }
            Err(e) => self.fail(&e),
        }
    }

    /// A close has begun: the session gets no more input (it ends and flushes), and the wait for the other side starts.
    fn begin_close(&mut self) {
        self.closing.get_or_insert_with(Instant::now);
        self.tx = None;
    }

    fn message(&mut self, message: Message) -> Step {
        self.last_rx = Instant::now();
        match message {
            Message::Text(text) => self.text(text.as_str()),
            Message::Binary(_) => self.refuse(
                CloseCode::Unsupported,
                "binary frames are not part of the wire",
            ),
            Message::Close(_) => {
                self.begin_close();
                Step::Go
            }
            _ => Step::Go,
        }
    }

    /// Give the session a text message as lines. Ignored once a close has begun; an empty message carries nothing.
    fn text(&mut self, text: &str) -> Step {
        let Some(tx) = self.tx.as_ref().filter(|_| self.closing.is_none()) else {
            return Step::Go;
        };
        if text.is_empty() {
            return Step::Go;
        }
        let mut line = text.as_bytes().to_vec();
        if line.last() != Some(&b'\n') {
            line.push(b'\n');
        }
        // blocks while the session's inbox is full (backpressure: the client's data waits in TCP); ends when it is gone
        if tx.send(line).is_err() {
            Step::End
        } else {
            Step::Go
        }
    }

    /// Nothing arrived for a tick: the close wait, the idle time and the ping.
    fn tick(&mut self) -> Step {
        let now = Instant::now();
        if let Some(since) = self.closing {
            // push out a close that could not be written at once
            let _ = lock(&self.ws).flush();
            return if now.duration_since(since) >= self.settings.close_wait {
                Step::End
            } else {
                Step::Go
            };
        }
        if now.duration_since(self.last_rx) >= self.settings.idle {
            return self.refuse(CloseCode::Away, "idle");
        }
        if now.duration_since(self.last_ping) >= self.settings.ping {
            self.last_ping = now;
            let mut ws = lock(&self.ws);
            let sent = ws.write(Message::Ping(Default::default()));
            let flushed = sent.and_then(|()| ws.flush());
            return match flushed {
                Err(WsError::Io(e)) if would_block(&e) => Step::Go,
                Err(_) => Step::End,
                Ok(()) => Step::Go,
            };
        }
        Step::Go
    }

    /// Close with a status and wait for the other side, for at most the close wait.
    fn refuse(&mut self, code: CloseCode, reason: &'static str) -> Step {
        self.begin_close();
        let _ = lock(&self.ws).close(Some(CloseFrame {
            code,
            reason: reason.into(),
        }));
        Step::Go
    }

    /// A read failed: the connection is over, or the client broke a rule and is told which.
    fn fail(&mut self, e: &WsError) -> Step {
        if self.closing.is_some() {
            return Step::End;
        }
        match e {
            WsError::ConnectionClosed | WsError::AlreadyClosed | WsError::Io(_) => Step::End,
            WsError::Protocol(ProtocolError::ResetWithoutClosingHandshake) => Step::End,
            WsError::Capacity(_) => self.refuse(CloseCode::Size, "message too big"),
            WsError::Utf8(_) => self.refuse(CloseCode::Invalid, "text is not UTF-8"),
            WsError::AttackAttempt => self.refuse(CloseCode::Policy, "refused"),
            _ => self.refuse(CloseCode::Protocol, "protocol error"),
        }
    }
}

/// The session's output: a frame line is a text message, written and flushed before the next one, and waited on for at
/// most the stall time (the lock is not held while waiting, so the reader goes on).
struct WsWriter {
    ws: Socket,
    fd: RawFd,
    closed: Arc<AtomicBool>,
    stall: Duration,
    pending: Vec<u8>,
}

impl WsWriter {
    fn send(&self, line: &[u8]) -> io::Result<()> {
        let message = Message::text(String::from_utf8_lossy(line).into_owned());
        match lock(&self.ws).write(message) {
            Ok(()) => {}
            Err(WsError::Io(e)) if would_block(&e) => {}
            Err(e) => return Err(io::Error::other(e.to_string())),
        }
        let until = Instant::now() + self.stall;
        loop {
            // the lock is let go before anything else takes it
            let flushed = lock(&self.ws).flush();
            match flushed {
                Ok(()) => return self.after_sent(line),
                Err(WsError::Io(e)) if would_block(&e) => {}
                Err(e) => return Err(io::Error::other(e.to_string())),
            }
            if self.closed.load(Ordering::SeqCst) || Instant::now() >= until {
                return Err(io::Error::other("the client stopped reading"));
            }
            wait_fd(self.fd, libc::POLLOUT);
        }
    }
}

impl WsWriter {
    /// A frame that says the credential ended (revoked or expired) was sent: the socket is closed with 1008, so it does
    /// not idle on with heartbeats for a member who may no longer act.
    fn after_sent(&self, line: &[u8]) -> io::Result<()> {
        if ends_credential(line) {
            let frame = CloseFrame {
                code: CloseCode::Policy,
                reason: "the credential is no longer valid".into(),
            };
            let _ = lock(&self.ws).close(Some(frame));
            self.closed.store(true, Ordering::SeqCst);
        }
        Ok(())
    }
}

/// Whether a frame line is the `refusal` of rule `credential`: what the gateway answers once a credential was revoked or
/// expired. A cheap look at the text first, so a message of a megabyte is never parsed for this.
fn ends_credential(line: &[u8]) -> bool {
    if line.len() > 1024 || !line.windows(12).any(|w| w == b"\"credential\"") {
        return false;
    }
    let Ok(frame) = serde_json::from_slice::<serde_json::Value>(line) else {
        return false;
    };
    frame["type"] == "refusal" && frame["rule"] == "credential"
}

impl Write for WsWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(buf);
        while let Some(at) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=at).collect();
            self.send(&line[..at])?;
        }
        if self.pending.len() > 2 * MAX_LINE_BYTES {
            return Err(io::Error::other("a frame is over the size limit"));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Send the `101` and put the connection in the hands of the protocol layer: non-blocking, with its limits.
fn upgraded(mut stream: Stream, accept: &str) -> Option<(Socket, RawFd)> {
    let answer = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    stream.write_all(answer.as_bytes()).ok()?;
    stream.flush().ok()?;
    let _ = stream.set_write_timeout(None);
    let _ = stream.set_read_timeout(None);
    stream.set_nonblocking(true).ok()?;
    let fd = stream.as_raw_fd();
    let ws = WebSocket::from_raw_socket(stream, Role::Server, Some(config()));
    Some((Arc::new(Mutex::new(ws)), fd))
}

/// What the session does when it ends: a close frame, then the reader takes it from there.
fn closer_of(ws: &Socket, closed: &Arc<AtomicBool>) -> Closer {
    let (ws, closed) = (ws.clone(), closed.clone());
    Box::new(move || {
        closed.store(true, Ordering::SeqCst);
        let code = Some(CloseFrame {
            code: CloseCode::Normal,
            reason: "".into(),
        });
        let _ = lock(&ws).close(code);
    })
}

/// Answer the upgrade and run the session over the socket until it ends.
fn connection(stream: Stream, who: &Principal, shared: &Shared, accept: &str) {
    let Ok(closer_end) = stream.try_clone() else {
        return;
    };
    let Some((ws, fd)) = upgraded(stream, accept) else {
        return;
    };
    let (closed, done) = (Arc::new(AtomicBool::new(false)), AtomicBool::new(false));
    let (tx, rx) = sync_channel::<Vec<u8>>(INBOX);
    let settings = &shared.ws;
    std::thread::scope(|scope| {
        // fires on every way out of this closure, a panic included, before the scope waits for the reader
        let teardown = Teardown(closer_end);
        let now = Instant::now();
        let reader = Reader {
            ws: ws.clone(),
            fd,
            tx: Some(tx),
            settings,
            closed: closed.clone(),
            last_rx: now,
            last_ping: now,
            closing: None,
            born: now,
        };
        scope.spawn(|| {
            let _done = Done(&done);
            reader.run();
        });
        let writer = WsWriter {
            ws: ws.clone(),
            fd,
            closed: closed.clone(),
            stall: settings.stall.unwrap_or(shared.config.stall),
            pending: Vec::new(),
        };
        let closer = closer_of(&ws, &closed);
        let input: Box<dyn BufRead + Send> = Box::new(io::BufReader::new(Inbox::new(rx, closed)));
        let session = Session::over(&shared.backend, who, &shared.config);
        let _ = session.serve_guarded(input, writer, Some(closer));
        wait_until(&done, settings.close_wait);
        teardown.fire();
    });
}

/// Wait for `flag` for at most `limit`.
fn wait_until(flag: &AtomicBool, limit: Duration) {
    let until = Instant::now() + limit;
    while !flag.load(Ordering::SeqCst) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
#[path = "wire_ws_tests.rs"]
mod tests;
