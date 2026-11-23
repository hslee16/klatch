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

use crate::loopback::{self, Budget, Head, Limits, Pending, Response, Stream};
use crate::wire_sse::SseWriter;
use anyhow::{Context, Result, bail};
use klatch::domain::Principal;
use klatch::gateway::{Identity, LocalIdentity, PeerEvidence, SharedBackend, SharedGateway};
use klatch::session::{Backend, Closer, Session, SessionConfig};
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
    /// Forwarded identity headers seen and ignored (#110): names and counts only.
    forwarded: crate::forwarded::Ignored,
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

/// The routes of the server: each has the one method it takes.
#[derive(Clone, Copy)]
enum Route {
    Stream,
    Frames,
    Mcp,
}

impl Route {
    fn of(path: &str) -> Option<Route> {
        match path {
            STREAM_PATH => Some(Route::Stream),
            FRAMES_PATH => Some(Route::Frames),
            crate::mcp_http::PATH => Some(Route::Mcp),
            _ => None,
        }
    }

    fn method(self) -> &'static str {
        match self {
            Route::Stream => "GET",
            Route::Frames | Route::Mcp => "POST",
        }
    }
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
    shared
        .forwarded
        .note(head.headers.iter().map(|(n, _)| n.as_str()));
    let Some(route) = Route::of(&head.path) else {
        return reply(&mut stream, &Response::new(404, "not found"));
    };
    if head.method != route.method() {
        return reply(&mut stream, &method_not_allowed(route.method()));
    }
    let budget = Budget {
        limits: &limits,
        deadline,
    };
    match route {
        Route::Stream => open_session(stream, &head, shared),
        Route::Frames => {
            let pending = Pending {
                stream: &mut stream,
                head,
                first,
                budget,
            };
            let response = post_frames(pending, shared);
            reply(&mut stream, &response);
        }
        Route::Mcp => {
            let pending = Pending {
                stream: &mut stream,
                head,
                first,
                budget,
            };
            let bytes = crate::mcp_http::post(pending, shared);
            crate::mcp_http::finish(&mut stream, &bytes);
        }
    }
}

/// `GET /v1/stream` for whoever the credential names; everyone else gets the one `401`.
fn open_session(mut stream: Stream, head: &Head, shared: &Shared) {
    match authenticate(&shared.backend, head) {
        Some(who) => stream_session(stream, &who, shared),
        None => reply(&mut stream, &unauthorized()),
    }
}

fn method_not_allowed(allow: &'static str) -> Response {
    let mut r = Response::new(405, "method not allowed");
    r.allow = Some(allow);
    r
}

/// The input of the member's own session `id`, or `None` for one that is another member's or never was.
fn session_input(shared: &Shared, id: &str, member: &str) -> Option<SyncSender<Vec<u8>>> {
    let sessions = shared.sessions.lock().unwrap_or_else(|e| e.into_inner());
    // another member's session is as absent as one that never was
    sessions
        .get(id)
        .filter(|e| is_session_id(id) && e.member == member)
        .map(|e| e.tx.clone())
}

/// `POST /v1/frames`: the credential, then the session, then the body, in that order, so nothing is read from a caller
/// who is not one and nothing is said about a session that is not theirs.
fn post_frames(mut req: Pending, shared: &Shared) -> Response {
    let Some(who) = authenticate(&shared.backend, &req.head) else {
        return unauthorized();
    };
    // a body over the limit is refused before it is read, and before anything is said about the session
    if req.declares_too_large() {
        return loopback::Fault::BodyTooLarge.response().unwrap();
    }
    let Some(id) = req.head.header(SESSION_HEADER) else {
        return Response::new(400, "x-klatch-session is required");
    };
    let Some(tx) = session_input(shared, id, &who.member) else {
        return Response::new(404, "no such session");
    };
    match req.body() {
        Ok(body) => queue_frames(&tx, body),
        Err(refusal) => refusal,
    }
}

/// Put the posted frames (one line each, a missing final newline added) in the session's input.
fn queue_frames(tx: &SyncSender<Vec<u8>>, mut body: Vec<u8>) -> Response {
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

/// A session's place in the registry: removing it ends what `POST /v1/frames` can reach.
#[derive(Clone)]
struct Registration {
    sessions: Sessions,
    id: String,
}

impl Registration {
    fn remove(&self) {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
    }
}

/// Register a new session of `who`: its registration, and the receiving end of its input.
fn register(shared: &Shared, id: &str, who: &Principal) -> (Registration, Receiver<Vec<u8>>) {
    let (tx, rx) = sync_channel::<Vec<u8>>(INBOX);
    let entry = Entry {
        member: who.member.clone(),
        tx,
    };
    let sessions = shared.sessions.clone();
    sessions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.to_string(), entry);
    let id = id.to_string();
    (Registration { sessions, id }, rx)
}

/// The head of the event stream's answer.
fn stream_head(id: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: close\r\nX-Accel-Buffering: no\r\nX-Klatch-Session: {id}\r\n\r\n"
    )
}

/// The three handles of one stream's connection, each with its own job.
struct Handles {
    out: Stream,
    watch: Stream,
    shut: Stream,
}

/// Clone the connection three ways and send the head; `None` when either fails.
fn open_stream(stream: &mut Stream, id: &str) -> Option<Handles> {
    let handles = Handles {
        out: stream.try_clone().ok()?,
        watch: stream.try_clone().ok()?,
        shut: stream.try_clone().ok()?,
    };
    stream
        .write_all(stream_head(id).as_bytes())
        .and_then(|()| stream.flush())
        .ok()?;
    Some(handles)
}

/// Read the stream for nothing but its end: a client that goes away ends the session without waiting for the next write
/// to fail.
fn watch_for_hangup(mut watch: Stream, gone: &AtomicBool) {
    let mut sink = [0u8; 256];
    while matches!(watch.read(&mut sink), Ok(n) if n > 0) {}
    gone.store(true, Ordering::SeqCst);
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
    let (registration, rx) = register(shared, &id, who);
    let Some(Handles { out, watch, shut }) = open_stream(&mut stream, &id) else {
        registration.remove();
        return;
    };
    let _ = stream.set_read_timeout(None);
    let _ = stream.set_write_timeout(None);
    let closed = Arc::new(AtomicBool::new(false));
    let gone = closed.clone();
    let closer: Closer = {
        let closed = closed.clone();
        Box::new(move || {
            closed.store(true, Ordering::SeqCst);
            registration.remove();
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
        scope.spawn(move || watch_for_hangup(watch, &gone));
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
fn announce_listening(out: &mut dyn Write, addr: &dyn std::fmt::Display, max_connections: usize) {
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
        forwarded: Default::default(),
    };
    loopback::serve_concurrent(
        &listener,
        &crate::daemon::STOP,
        opts.max_connections,
        loopback::Callbacks {
            handler: &|stream| serve_connection(stream, &shared),
            excess: &refuse_excess,
        },
    );
    eprintln!("{}", shared.forwarded.summary());
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
        announce_listening(&mut out, &"127.0.0.1:43210", 8);
        assert_eq!(out.writes, 1);
        let text = String::from_utf8(out.bytes).unwrap();
        assert!(text.starts_with("klatch-server: http listening on 127.0.0.1:43210 "));
        assert!(text.ends_with('\n'));
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
