//! `klatch-server http --listen 127.0.0.1:PORT` (#1059, phase 3 of epic #1056; the HTTP transport of #109): the klatch
//! wire protocol (PROTOCOL.md) over HTTP and server-sent events, on loopback only. It is not a second protocol: a stream
//! runs the same session loop as `klatch serve --stdio` and the daemon (`Session` over the one shared gateway), and its
//! frames are the same JSON lines, so a frame means here what it means there.
//!
//! - `GET /v1/stream` opens one session. The answer is `text/event-stream`; its header `X-Klatch-Session` names the
//!   session, and every frame the session writes (`hello` first) is one event: `event: <frame type>`, `data: <the frame
//!   line>`, and for a `message` frame `id: <room>:<seq>`. Closing the connection ends the session.
//! - `POST /v1/frames` with `X-Klatch-Session` writes the frames in the body (JSON lines, as on stdio) into that
//!   session and answers `202` with nothing: the `response` to a request arrives on the stream, as the daemon's does on
//!   its connection. A session that has ended is `404`.
//! - Resume needs nothing new: sequence numbers are the store's, so a new stream sends `subscribe` with `from_seq` set
//!   to the last `seq` it saw and gets the same messages with the same numbers.
//!
//! **Who is calling.** Both routes need `Authorization: Bearer <token>` and `X-Klatch-Member: <member>`: a worker
//! credential the human issued, checked live (token, expiry, revocation) by `verify_credential`, exactly as the
//! daemon's `identify` and a key bundle are. The result is the evidence `PeerEvidence::Credential`, a verified member.
//! There is no unauthenticated or typed-name tier over HTTP: nothing here can tell who a peer is except the credential,
//! and every refusal is the same `401 unauthorized` (it never says which part was wrong). The token is a header value
//! only: it is never logged, echoed or put in a URL.
//!
//! **Limits.** Loopback only (`loopback::bind_loopback` refuses anything else, naming #157 and #161); 16 KiB of
//! headers, 1 MiB bodies, a 5 second deadline for a request; at most `--max-connections` open connections, the excess
//! answered `503`; the session's own bounded queue and stall limit (#147) for a stream whose reader stopped. One
//! connection serves one exchange (`Connection: close`), so a client opens one connection for its stream and one per POST.
//!
//! This module names no socket type: the listener and the connection come from `loopback`.

use crate::loopback::{self, Head, Limits, Response, Stream};
use anyhow::{Context, Result, bail};
use klatch::domain::Principal;
use klatch::gateway::{Identity, LocalIdentity, PeerEvidence, SharedBackend, SharedGateway};
use klatch::protocol::MAX_LINE_BYTES;
use klatch::session::{Backend, Closer, Session, SessionConfig};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{self, BufRead, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const STREAM_PATH: &str = "/v1/stream";
pub const FRAMES_PATH: &str = "/v1/frames";
pub const MEMBER_HEADER: &str = "x-klatch-member";
pub const SESSION_HEADER: &str = "x-klatch-session";

/// Concurrent connections served by default.
pub const DEFAULT_MAX_CONNECTIONS: usize = 32;
/// Posted bodies a session may have waiting to be read by its session loop.
const INBOX: usize = 16;

/// What `klatch-server http` was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub listen: String,
    pub max_connections: usize,
}

/// Read the options after `http`. The address has no default: a server never picks a place by itself.
pub fn parse(args: &[&str]) -> Result<Options> {
    let mut listen = None;
    let mut max_connections = DEFAULT_MAX_CONNECTIONS;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| -> Result<&str> {
            it.next()
                .copied()
                .with_context(|| format!("{name} needs a value"))
        };
        match *a {
            "--listen" => listen = Some(value("--listen")?.to_string()),
            "--max-connections" => {
                max_connections = value("--max-connections")?
                    .parse()
                    .ok()
                    .filter(|n| *n > 0)
                    .context("--max-connections is a positive number")?;
            }
            other => bail!("unknown option {other} for http"),
        }
    }
    Ok(Options {
        listen: listen.context("http needs --listen 127.0.0.1:PORT")?,
        max_connections,
    })
}

// ---------------------------------------------------------------------------------------------------------------
// the frames of a session as server-sent events

/// One frame line (without its newline) as one server-sent event. The event name is the frame's type, a `message`
/// frame carries `id: <room>:<seq>`, and the data is the line exactly as the stdio session writes it.
pub fn sse_event(line: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(line);
    let frame: Option<Value> = serde_json::from_str(&text).ok();
    let mut out = String::new();
    if let Some(f) = &frame
        && let Some(t) = f["type"].as_str()
        && t.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
    {
        out.push_str(&format!("event: {t}\n"));
        if t == "message"
            && let (Some(room), Some(seq)) = (f["room"].as_str(), f["seq"].as_u64())
            && room.bytes().all(|b| (0x21..0x7f).contains(&b))
        {
            out.push_str(&format!("id: {room}:{seq}\n"));
        }
    }
    // a frame line holds no raw newline (JSON escapes them); a stray carriage return would end an SSE line early
    out.push_str(&format!("data: {}\n\n", text.replace('\r', "\\r")));
    out.into_bytes()
}

/// The output of a session: bytes in, whole lines out as events. The session writes one frame per write, but a line is
/// only an event once its newline has come.
pub struct SseWriter<W: Write> {
    out: W,
    pending: Vec<u8>,
}

impl<W: Write> SseWriter<W> {
    pub fn new(out: W) -> Self {
        SseWriter {
            out,
            pending: Vec::new(),
        }
    }
}

impl<W: Write> Write for SseWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(buf);
        while let Some(at) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=at).collect();
            let event = sse_event(&line[..at]);
            self.out.write_all(&event)?;
        }
        if self.pending.len() > 2 * MAX_LINE_BYTES {
            return Err(io::Error::other("a frame is over the size limit"));
        }
        self.out.flush()?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// the input of a session: what POSTs put in

/// The input side of a session, fed by `POST /v1/frames`. It ends when the stream's connection is gone (`closed`) or
/// the session is dropped from the registry (the sender is gone).
struct Inbox {
    rx: Receiver<Vec<u8>>,
    closed: Arc<AtomicBool>,
    buf: Vec<u8>,
    at: usize,
}

impl Read for Inbox {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.at < self.buf.len() {
                let n = out.len().min(self.buf.len() - self.at);
                out[..n].copy_from_slice(&self.buf[self.at..self.at + n]);
                self.at += n;
                return Ok(n);
            }
            if self.closed.load(Ordering::SeqCst) {
                return Ok(0);
            }
            match self.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(bytes) => {
                    self.buf = bytes;
                    self.at = 0;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(0),
            }
        }
    }
}

/// A session that is open: whose it is, and where its input goes.
struct Entry {
    member: String,
    tx: SyncSender<Vec<u8>>,
}

type Sessions = Arc<Mutex<HashMap<String, Entry>>>;

/// What every connection thread shares.
pub(crate) struct Shared {
    pub(crate) backend: SharedBackend,
    pub(crate) config: SessionConfig,
    sessions: Sessions,
}

/// 128 random bits as 32 lower-case hex digits: the name of a session. Not a secret that stands alone (every use also
/// needs the credential), but unguessable all the same.
fn new_session_id() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn is_session_id(s: &str) -> bool {
    s.len() == 32
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

// ---------------------------------------------------------------------------------------------------------------
// who is calling

/// The text of a bearer token in an `Authorization` value: the scheme is matched without regard to case, the token is
/// whatever follows one space.
fn bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

/// The verified principal a request's credential names, or `None` for every way of not having one.
pub(crate) fn authenticate(backend: &SharedBackend, head: &Head) -> Option<Principal> {
    let member = head.header(MEMBER_HEADER)?;
    let token = bearer_token(head.header("authorization")?)?;
    backend.verify_credential(member, token).ok()?;
    // the principal is built the way every credential becomes one
    LocalIdentity
        .resolve(&PeerEvidence::Credential {
            member: member.to_string(),
        })
        .ok()
}

pub(crate) fn unauthorized() -> Response {
    Response::new(401, "unauthorized")
}

// ---------------------------------------------------------------------------------------------------------------
// serving

/// Answer, then let the client finish what it was sending before the connection closes, so a refusal sent before the
/// body was read is not lost to a reset.
fn reply(stream: &mut Stream, response: &Response) {
    if response.status == 0 {
        return;
    }
    reply_bytes(stream, &response.bytes());
}

/// [`reply`] for an answer already built (the MCP route's JSON answers).
pub(crate) fn reply_bytes(stream: &mut Stream, bytes: &[u8]) {
    let _ = stream.write_all(bytes);
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let until = Instant::now() + Duration::from_secs(1);
    let mut sink = [0u8; 8192];
    let mut left = 4 * 1024 * 1024usize;
    while left > 0 && Instant::now() < until {
        let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
        match stream.read(&mut sink) {
            Ok(0) | Err(_) => break,
            Ok(n) => left = left.saturating_sub(n),
        }
    }
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

fn refuse_excess(mut stream: Stream) {
    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
    let _ = stream.write_all(&Response::new(503, "busy").bytes());
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

fn serve_connection(mut stream: Stream, shared: &Shared) {
    let limits = Limits::default();
    let deadline = Instant::now() + limits.deadline;
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let (head, first) = match loopback::read_head(&mut stream, &limits, deadline) {
        Ok(x) => x,
        Err(f) => {
            if let Some(r) = f.response() {
                reply(&mut stream, &r);
            }
            return;
        }
    };
    match (head.method.as_str(), head.path.as_str()) {
        ("GET", STREAM_PATH) => match authenticate(&shared.backend, &head) {
            Some(who) => stream_session(stream, &who, shared),
            None => reply(&mut stream, &unauthorized()),
        },
        ("POST", FRAMES_PATH) => {
            let response = post_frames(&mut stream, head, first, &limits, deadline, shared);
            reply(&mut stream, &response);
        }
        ("POST", crate::mcp_http::PATH) => {
            let bytes = crate::mcp_http::post(&mut stream, head, first, &limits, deadline, shared);
            crate::mcp_http::finish(&mut stream, &bytes);
        }
        (_, crate::mcp_http::PATH) => reply(&mut stream, &method_not_allowed("POST")),
        (_, STREAM_PATH) => reply(&mut stream, &method_not_allowed("GET")),
        (_, FRAMES_PATH) => reply(&mut stream, &method_not_allowed("POST")),
        _ => reply(&mut stream, &Response::new(404, "not found")),
    }
}

fn method_not_allowed(allow: &'static str) -> Response {
    let mut r = Response::new(405, "method not allowed");
    r.allow = Some(allow);
    r
}

/// `POST /v1/frames`: the credential, then the session, then the body, in that order, so nothing is read from a caller
/// who is not one and nothing is said about a session that is not theirs.
fn post_frames(
    stream: &mut Stream,
    head: Head,
    first: Vec<u8>,
    limits: &Limits,
    deadline: Instant,
    shared: &Shared,
) -> Response {
    let Some(who) = authenticate(&shared.backend, &head) else {
        return unauthorized();
    };
    // a body over the limit is refused before it is read, and before anything is said about the session
    if head.content_length.is_some_and(|n| n > limits.max_body) {
        return loopback::Fault::BodyTooLarge.response().unwrap();
    }
    let Some(id) = head.header(SESSION_HEADER) else {
        return Response::new(400, "x-klatch-session is required");
    };
    let tx = {
        let sessions = shared.sessions.lock().unwrap_or_else(|e| e.into_inner());
        match sessions.get(id) {
            // another member's session is as absent as one that never was
            Some(e) if is_session_id(id) && e.member == who.member => Some(e.tx.clone()),
            _ => None,
        }
    };
    let Some(tx) = tx else {
        return Response::new(404, "no such session");
    };
    let mut body = match head.content_length {
        Some(n) => match loopback::read_body(stream, n, first, limits, deadline) {
            Ok(b) => b,
            Err(f) => return f.response().unwrap_or_else(|| Response::new(0, "")),
        },
        None => return loopback::Fault::LengthRequired.response().unwrap(),
    };
    if body.is_empty() {
        return Response::new(400, "an empty body carries no frame");
    }
    if body.last() != Some(&b'\n') {
        body.push(b'\n');
    }
    match tx.try_send(body) {
        Ok(()) => Response::new(202, "accepted"),
        Err(TrySendError::Full(_)) => Response::new(429, "the session is busy; retry"),
        Err(TrySendError::Disconnected(_)) => Response::new(404, "no such session"),
    }
}

/// `GET /v1/stream`: register a session, answer with the event-stream head, and run the session loop until the
/// connection ends. The loop is the one every transport runs.
fn stream_session(mut stream: Stream, who: &Principal, shared: &Shared) {
    let Ok(id) = new_session_id() else {
        reply(
            &mut stream,
            &Response::new(503, "no randomness for a session"),
        );
        return;
    };
    let (tx, rx) = sync_channel::<Vec<u8>>(INBOX);
    let closed = Arc::new(AtomicBool::new(false));
    shared
        .sessions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(
            id.clone(),
            Entry {
                member: who.member.clone(),
                tx,
            },
        );
    let unregister = {
        let sessions = shared.sessions.clone();
        let id = id.clone();
        move || {
            sessions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
        }
    };
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: close\r\nX-Accel-Buffering: no\r\nX-Klatch-Session: {id}\r\n\r\n"
    );
    let (Ok(out), Ok(watch), Ok(shut)) =
        (stream.try_clone(), stream.try_clone(), stream.try_clone())
    else {
        unregister();
        return;
    };
    if stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.flush())
        .is_err()
    {
        unregister();
        return;
    }
    // the stream is read for nothing but its end: a client that goes away ends the session without waiting for the
    // next write to fail
    let _ = stream.set_read_timeout(None);
    let _ = stream.set_write_timeout(None);
    let gone = closed.clone();
    let closer: Closer = {
        let closed = closed.clone();
        Box::new(move || {
            closed.store(true, Ordering::SeqCst);
            unregister();
            let _ = shut.shutdown(std::net::Shutdown::Both);
        })
    };
    let input = io::BufReader::new(Inbox {
        rx,
        closed,
        buf: Vec::new(),
        at: 0,
    });
    std::thread::scope(|scope| {
        scope.spawn(move || {
            let mut watch = watch;
            let mut sink = [0u8; 256];
            while matches!(watch.read(&mut sink), Ok(n) if n > 0) {}
            gone.store(true, Ordering::SeqCst);
        });
        let session = Session::over(&shared.backend, who, &shared.config);
        let _ = session.serve_guarded(input_box(input), SseWriter::new(out), Some(closer));
    });
}

fn input_box(input: io::BufReader<Inbox>) -> Box<dyn BufRead + Send> {
    Box::new(input)
}

/// Write the readiness line as one complete write, newline included. `eprintln!` can reach the stream in several
/// writes, so a launcher polling the log could read half an address ("127.0.0.1:" or a port cut short) and parse it
/// (#1173, #1207). Launchers also wait for the newline before they trust the line.
fn announce_listening(out: &mut dyn Write, addr: &std::net::SocketAddr, max_connections: usize) {
    let line = format!(
        "klatch-server: http listening on {addr} (loopback only, up to {max_connections} connections), GET {STREAM_PATH}, POST {FRAMES_PATH} and POST {} (MCP)\n",
        crate::mcp_http::PATH
    );
    let _ = out.write_all(line.as_bytes());
    let _ = out.flush();
}

/// Run the server until SIGTERM or SIGINT. Order matters: the address is checked (a refusal ends the process before the
/// store is touched), then the store is opened (one that cannot be opened ends it before it listens), then the listener
/// is bound.
pub fn run(opts: &Options) -> Result<()> {
    loopback::check_loopback(&opts.listen)?;
    let gateway: SharedGateway = klatch::member::spawn_shared_gateway()
        .context("the server cannot open the store, so it does not start")?;
    let listener = loopback::bind_loopback(&opts.listen)?;
    crate::daemon::install_signal_handlers();
    announce_listening(
        &mut io::stderr().lock(),
        &listener.local_addr()?,
        opts.max_connections,
    );
    let shared = Shared {
        backend: gateway.session_backend(),
        config: SessionConfig::default(),
        sessions: Arc::default(),
    };
    loopback::serve_concurrent(
        &listener,
        &crate::daemon::STOP,
        opts.max_connections,
        &|stream| serve_connection(stream, &shared),
        &refuse_excess,
    );
    drop(shared);
    drop(gateway);
    eprintln!("klatch-server: http stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A writer that counts how many separate writes reach it.
    #[derive(Default)]
    struct Counting {
        bytes: Vec<u8>,
        writes: usize,
    }

    impl Write for Counting {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn the_listening_line_reaches_the_stream_in_one_write() {
        // A reader that polls the log must never see half of the line, so the whole line is one write (#1173, #1207).
        let mut out = Counting::default();
        let addr: std::net::SocketAddr = "127.0.0.1:43210".parse().unwrap();
        announce_listening(&mut out, &addr, 8);
        assert_eq!(out.writes, 1);
        let text = String::from_utf8(out.bytes).unwrap();
        assert!(text.starts_with("klatch-server: http listening on 127.0.0.1:43210 "));
        assert!(text.ends_with('\n'));
    }

    fn events(out: &[u8]) -> Vec<String> {
        String::from_utf8(out.to_vec())
            .unwrap()
            .split("\n\n")
            .filter(|e| !e.is_empty())
            .map(String::from)
            .collect()
    }

    #[test]
    fn an_event_carries_the_type_the_line_as_data_and_for_a_message_its_room_and_sequence_number() {
        let line = br#"{"v":1,"type":"message","room":"lobby","seq":42,"id":"m","sender":"a","body":"hi"}"#;
        let e = String::from_utf8(sse_event(line)).unwrap();
        assert_eq!(
            e,
            format!(
                "event: message\nid: lobby:42\ndata: {}\n\n",
                std::str::from_utf8(line).unwrap()
            )
        );
        let e = String::from_utf8(sse_event(br#"{"v":1,"type":"heartbeat","ts":5}"#)).unwrap();
        assert_eq!(
            e,
            "event: heartbeat\ndata: {\"v\":1,\"type\":\"heartbeat\",\"ts\":5}\n\n"
        );
        // no id for a frame that is not a message, none when the room is odd
        let e =
            String::from_utf8(sse_event(br#"{"type":"caught_up","room":"r","seq":3}"#)).unwrap();
        assert!(!e.contains("\nid:"), "{e}");
        let e = String::from_utf8(sse_event(
            "{\"type\":\"message\",\"room\":\"a b\\n\",\"seq\":1}".as_bytes(),
        ))
        .unwrap();
        assert!(!e.contains("\nid:"), "{e}");
    }

    #[test]
    fn a_line_that_is_not_json_is_still_one_data_line_and_never_breaks_the_event() {
        for bad in [&b"not json"[..], b"{\"type\":\"x y\"}", b"a\rb: c"] {
            let e = String::from_utf8(sse_event(bad)).unwrap();
            assert!(e.ends_with("\n\n"), "{e:?}");
            let lines: Vec<&str> = e.trim_end_matches('\n').split('\n').collect();
            assert!(
                lines
                    .iter()
                    .all(|l| l.starts_with("data: ") || l.starts_with("event: ")),
                "{e:?}"
            );
            assert_eq!(
                lines.iter().filter(|l| l.starts_with("data: ")).count(),
                1,
                "{e:?}"
            );
            assert!(!e.contains('\r'), "{e:?}");
        }
    }

    #[test]
    fn the_writer_makes_an_event_of_each_whole_line_and_holds_a_partial_one() {
        let mut out = Vec::new();
        {
            let mut w = SseWriter::new(&mut out);
            w.write_all(b"{\"type\":\"hel").unwrap();
            w.write_all(b"lo\"}\n{\"type\":\"heartbeat\"}\n{\"typ")
                .unwrap();
        }
        let evs = events(&out);
        assert_eq!(evs.len(), 2, "{evs:?}");
        assert!(evs[0].starts_with("event: hello\n"));
        assert!(evs[1].starts_with("event: heartbeat\n"));
    }

    #[test]
    fn the_writer_refuses_a_line_with_no_end_that_grows_without_bound() {
        let mut w = SseWriter::new(Vec::new());
        let chunk = vec![b'x'; MAX_LINE_BYTES];
        assert!(w.write_all(&chunk).is_ok());
        assert!(w.write_all(&chunk).is_ok());
        assert!(w.write_all(&chunk).is_err());
    }

    #[test]
    fn the_inbox_hands_over_posted_bytes_in_order_and_ends_when_closed_or_dropped() {
        let (tx, rx) = sync_channel(4);
        let closed = Arc::new(AtomicBool::new(false));
        let mut r = io::BufReader::new(Inbox {
            rx,
            closed: closed.clone(),
            buf: Vec::new(),
            at: 0,
        });
        tx.send(b"a\nb".to_vec()).unwrap();
        tx.send(b"c\n".to_vec()).unwrap();
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        assert_eq!(line, "a\n");
        line.clear();
        r.read_line(&mut line).unwrap();
        assert_eq!(line, "bc\n");
        closed.store(true, Ordering::SeqCst);
        line.clear();
        assert_eq!(r.read_line(&mut line).unwrap(), 0);
        // a dropped sender ends it too
        let (tx, rx) = sync_channel::<Vec<u8>>(1);
        drop(tx);
        let mut r = Inbox {
            rx,
            closed: Arc::default(),
            buf: Vec::new(),
            at: 0,
        };
        assert_eq!(r.read(&mut [0u8; 4]).unwrap(), 0);
    }

    #[test]
    fn session_ids_are_32_hex_digits_and_differ() {
        let (a, b) = (new_session_id().unwrap(), new_session_id().unwrap());
        assert!(is_session_id(&a) && is_session_id(&b));
        assert_ne!(a, b);
        for bad in ["", "abc", &"g".repeat(32), &"A".repeat(32), &"a".repeat(33)] {
            assert!(!is_session_id(bad), "{bad}");
        }
    }

    #[test]
    fn a_bearer_value_has_the_scheme_and_one_token() {
        assert_eq!(bearer_token("Bearer abc"), Some("abc"));
        assert_eq!(bearer_token("bearer abc"), Some("abc"));
        for bad in ["", "Bearer", "Bearer ", "Basic abc", "abc", "Bearerabc"] {
            assert_eq!(bearer_token(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn options_need_an_address_and_take_one_limit() {
        assert!(parse(&[]).is_err());
        assert!(parse(&["--listen"]).is_err());
        assert!(parse(&["--listen", "127.0.0.1:1", "--bogus"]).is_err());
        assert!(parse(&["--listen", "127.0.0.1:1", "--max-connections", "0"]).is_err());
        let o = parse(&["--listen", "127.0.0.1:1", "--max-connections", "3"]).unwrap();
        assert_eq!(o.max_connections, 3);
        assert_eq!(o.listen, "127.0.0.1:1");
        assert_eq!(
            parse(&["--listen", "[::1]:2"]).unwrap().max_connections,
            DEFAULT_MAX_CONNECTIONS
        );
    }
}
