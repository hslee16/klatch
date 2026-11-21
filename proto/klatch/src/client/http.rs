//! The Rust client over `klatch-server http` (#1122, feature `client-http`): the wire protocol as server-sent events and
//! posted frames, the same transport as `Client.connect_http` (Python) and `Client.connectHttp` (TypeScript, #1124). This
//! is the only file of the `klatch` crate that names a network socket type, and only `TcpStream`: it connects to a port
//! on this machine and listens for nothing. The feature is in no default feature (`cli`, `client` and `default` never
//! turn it on), so the shipped `klatch` binary does not contain this code; `tests/no_telemetry.rs` pins both facts.
//!
//! Only plain `http` to a loopback address is accepted, and anything else is refused before a socket is opened. The
//! stream is `GET /v1/stream` (the reply carries `X-Klatch-Session`, then one frame per event in its `data:` line) and
//! each request is a `POST /v1/frames` carrying that header; the answer comes on the stream, as it does on the
//! daemon's connection. Both carry `Authorization: Bearer TOKEN` and `X-Klatch-Member`. When the stream ends without a
//! close, the reader opens a new one, drops the `hello` of the new session and resubscribes every room that was
//! subscribed from the last `seq` seen in it, so the caller sees no gap and no repeat.

use super::{ApiError, Client, Error, Identity, Result};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The longest line of a reply that is accepted: a frame is far smaller, and a peer that never ends a line is not served.
const MAX_LINE: u64 = 4 * 1024 * 1024;
/// How often a post is tried again when the session's inbox is full (it drains, so it is tried again soon).
const POST_ATTEMPTS: u32 = 40;
const POST_RETRY: Duration = Duration::from_millis(50);

/// How a connect behaves. Build it with [`HttpOptions::new`] and the setters; the defaults are a 10 second connect and
/// post deadline, and 5 reconnect attempts 200 milliseconds apart (growing by that much each time).
#[derive(Clone, Debug)]
pub struct HttpOptions {
    connect_timeout: Duration,
    reconnect_attempts: u32,
    reconnect_delay: Duration,
    fault: Option<HttpFault>,
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpOptions {
    pub fn new() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            reconnect_attempts: 5,
            reconnect_delay: Duration::from_millis(200),
            fault: None,
        }
    }

    /// The deadline of opening a connection, of sending a post and of reading its status.
    pub fn connect_timeout(mut self, t: Duration) -> Self {
        self.connect_timeout = t;
        self
    }

    /// How many times a stream that ended is reopened before the session is over (0: never).
    pub fn reconnect_attempts(mut self, n: u32) -> Self {
        self.reconnect_attempts = n;
        self
    }

    /// The wait before the first reopen; the wait before the nth is n times this.
    pub fn reconnect_delay(mut self, d: Duration) -> Self {
        self.reconnect_delay = d;
        self
    }

    /// A test hook: lets a test break the stream without depending on how the platform reports a reset.
    pub fn fault(mut self, fault: &HttpFault) -> Self {
        self.fault = Some(fault.clone());
        self
    }
}

/// A test hook (see [`HttpOptions::fault`]): the reader fails with a connection reset at its next line, as if the
/// stream had been broken, and the client reconnects. Nothing in a normal session uses it.
#[derive(Clone, Debug, Default)]
pub struct HttpFault(Arc<AtomicBool>);

impl HttpFault {
    pub fn new() -> Self {
        Self::default()
    }

    /// The next line the reader would read fails with `ConnectionReset`.
    pub fn fail_next_read(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    fn take(&self) -> bool {
        self.0.swap(false, Ordering::SeqCst)
    }
}

/// A loopback server address from a url, or why it is not one. Pure: nothing is resolved and nothing connects.
fn parse_url(url: &str) -> std::result::Result<SocketAddr, String> {
    let refuse = || {
        format!(
            "the url is http://127.0.0.1:PORT (plain http, loopback address only, no path or credentials), not {url:?}"
        )
    };
    let rest = url.strip_prefix("http://").ok_or_else(refuse)?;
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    let (host, port) = if let Some(inner) = rest.strip_prefix('[') {
        let (h, p) = inner.split_once("]:").ok_or_else(refuse)?;
        (format!("[{h}]"), p)
    } else {
        let (h, p) = rest.rsplit_once(':').ok_or_else(refuse)?;
        (h.to_string(), p)
    };
    if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
        return Err(refuse());
    }
    let port: u16 = port.parse().map_err(|_| refuse())?;
    if port == 0 {
        return Err(refuse());
    }
    let ip = match host.as_str() {
        "127.0.0.1" | "localhost" => Ipv4Addr::LOCALHOST.into(),
        "[::1]" => Ipv6Addr::LOCALHOST.into(),
        _ => return Err(refuse()),
    };
    Ok(SocketAddr::new(ip, port))
}

/// A header value that cannot split a request: visible ASCII only, no space, not empty.
fn header_safe(v: &str) -> bool {
    !v.is_empty() && v.bytes().all(|b| b.is_ascii_graphic())
}

/// The state both halves of the transport share.
struct Shared {
    addr: SocketAddr,
    member: String,
    token: String,
    opts: HttpOptions,
    closed: AtomicBool,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    session: String,
    /// A handle on the open stream, to shut it from another thread.
    stream: Option<TcpStream>,
    /// room -> the `from_seq` to resubscribe from
    subs: HashMap<String, u64>,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    fn connect(&self) -> io::Result<TcpStream> {
        let s = TcpStream::connect_timeout(&self.addr, self.opts.connect_timeout)?;
        let _ = s.set_nodelay(true);
        Ok(s)
    }

    fn head(&self, method: &str, path: &str, extra: &str) -> String {
        format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nX-Klatch-Member: {}\r\n{extra}Connection: close\r\n\r\n",
            self.addr, self.token, self.member
        )
    }

    /// Open a session: the stream, past its head, and its session id. A 401 is `identity.refused`.
    fn open_stream(&self) -> Result<BufReader<TcpStream>> {
        let mut s = self
            .connect()
            .map_err(|e| Error::Session(format!("could not connect to {}: {e}", self.addr)))?;
        let session_err = |e: io::Error| Error::Session(format!("the stream failed to open: {e}"));
        s.set_write_timeout(Some(self.opts.connect_timeout))
            .map_err(session_err)?;
        s.set_read_timeout(Some(self.opts.connect_timeout))
            .map_err(session_err)?;
        s.write_all(
            self.head("GET", "/v1/stream", "Accept: text/event-stream\r\n")
                .as_bytes(),
        )
        .map_err(session_err)?;
        let handle = s.try_clone().map_err(session_err)?;
        let mut r = BufReader::new(s);
        let (status, headers) = read_head(&mut r).map_err(session_err)?;
        match status {
            200 => {}
            401 => {
                return Err(Error::Api(ApiError {
                    code: "identity.refused".into(),
                    message: "the server refused the credential".into(),
                    refusals: Vec::new(),
                }));
            }
            other => {
                return Err(Error::Session(format!(
                    "the server answered {other} to the stream"
                )));
            }
        }
        let session = headers
            .get("x-klatch-session")
            .filter(|v| !v.is_empty())
            .ok_or_else(|| Error::Protocol("the stream has no X-Klatch-Session header".into()))?
            .clone();
        // heartbeats keep a healthy stream busy; shutting the socket is what unblocks the read
        r.get_ref().set_read_timeout(None).map_err(session_err)?;
        let mut st = self.lock();
        if self.is_closed() {
            let _ = handle.shutdown(Shutdown::Both);
            return Err(Error::Session("the session is closed".into()));
        }
        st.session = session;
        st.stream = Some(handle);
        Ok(r)
    }

    fn release(&self) {
        if let Some(s) = self.lock().stream.take() {
            let _ = s.shutdown(Shutdown::Both);
        }
    }

    fn shut(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.release();
    }

    /// POST one frame line to the session, trying again while the inbox is full.
    fn post_line(&self, line: &str) -> io::Result<()> {
        for attempt in 0..POST_ATTEMPTS {
            if self.is_closed() {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "the session is closed",
                ));
            }
            let session = self.lock().session.clone();
            let status = self.post_once(&session, line)?;
            match status {
                202 => return Ok(()),
                429 if attempt + 1 < POST_ATTEMPTS => std::thread::sleep(POST_RETRY),
                other => {
                    return Err(io::Error::other(format!(
                        "the server answered {other} to a post"
                    )));
                }
            }
        }
        Err(io::Error::other("the session's inbox stayed full"))
    }

    fn post_once(&self, session: &str, line: &str) -> io::Result<u16> {
        let mut s = self.connect()?;
        s.set_write_timeout(Some(self.opts.connect_timeout))?;
        s.set_read_timeout(Some(self.opts.connect_timeout))?;
        let extra = format!(
            "X-Klatch-Session: {session}\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\n",
            line.len()
        );
        s.write_all(self.head("POST", "/v1/frames", &extra).as_bytes())?;
        s.write_all(line.as_bytes())?;
        let (status, _) = read_head(&mut BufReader::new(&s))?;
        Ok(status)
    }

    /// Remember what the session subscribes to, to resubscribe after a reconnect.
    fn note_sent(&self, line: &str) {
        let Ok(Value::Object(frame)) = serde_json::from_str::<Value>(line) else {
            return;
        };
        if frame.get("type").and_then(Value::as_str) != Some("request") {
            return;
        }
        let params = frame.get("params");
        let Some(room) = params
            .and_then(|p| p.get("room"))
            .and_then(Value::as_str)
            .map(String::from)
        else {
            return;
        };
        match frame.get("method").and_then(Value::as_str) {
            Some("subscribe") => {
                let from = params
                    .and_then(|p| p.get("from_seq"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                self.lock().subs.insert(room, from);
            }
            Some("unsubscribe") => {
                self.lock().subs.remove(&room);
            }
            _ => {}
        }
    }

    fn note_seen(&self, data: &str) {
        let Ok(frame) = serde_json::from_str::<Value>(data) else {
            return;
        };
        if frame["type"] != "message" {
            return;
        }
        if let (Some(room), Some(seq)) = (frame["room"].as_str(), frame["seq"].as_u64())
            && let Some(from) = self.lock().subs.get_mut(room)
        {
            *from = seq;
        }
    }
}

/// The status and the headers (names lower-cased) of a reply.
fn read_head<R: BufRead>(r: &mut R) -> io::Result<(u16, HashMap<String, String>)> {
    let line = read_line(r)?.ok_or_else(|| io::ErrorKind::UnexpectedEof)?;
    let status = line
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not an http reply"))?;
    let mut headers = HashMap::new();
    loop {
        let h = read_line(r)?.ok_or_else(|| io::ErrorKind::UnexpectedEof)?;
        if h.is_empty() {
            return Ok((status, headers));
        }
        if let Some((n, v)) = h.split_once(':') {
            headers.insert(n.to_ascii_lowercase(), v.trim().to_string());
        }
    }
}

/// One line without its line end, at most `MAX_LINE` bytes; `None` at the end of the stream.
fn read_line<R: BufRead>(r: &mut R) -> io::Result<Option<String>> {
    let mut buf = Vec::new();
    let n = r.by_ref().take(MAX_LINE + 1).read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(None);
    }
    if buf.last() != Some(&b'\n') {
        return if n as u64 > MAX_LINE {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "a line is too long",
            ))
        } else {
            Ok(None) // the stream ended inside a line: that line was never finished
        };
    }
    while matches!(buf.last(), Some(b'\n' | b'\r')) {
        buf.pop();
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

/// The stream as a `Read` of frame lines (the `data:` of each event, then a newline), across reconnects.
struct SseReader {
    shared: Arc<Shared>,
    body: Option<BufReader<TcpStream>>,
    out: Vec<u8>,
    pos: usize,
    /// The first event of a session after a reconnect is its `hello`, which the caller already has.
    drop_hello: bool,
    rid: u64,
}

impl SseReader {
    /// The `data:` of the next event, or an error when the stream ended or broke.
    fn event(&mut self) -> io::Result<String> {
        let mut data: Vec<String> = Vec::new();
        loop {
            if self.shared.opts.fault.as_ref().is_some_and(HttpFault::take) {
                return Err(io::ErrorKind::ConnectionReset.into());
            }
            let body = self
                .body
                .as_mut()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotConnected))?;
            let Some(line) = read_line(body)? else {
                return Err(io::ErrorKind::UnexpectedEof.into());
            };
            if line.is_empty() {
                if !data.is_empty() {
                    return Ok(data.join("\n"));
                }
            } else if let Some(d) = line.strip_prefix("data:") {
                data.push(d.strip_prefix(' ').unwrap_or(d).to_string());
            }
            // a comment (heartbeat), `event:`, `id:` and `retry:` lines carry nothing the frame does not
        }
    }

    /// The next frame line, reconnecting when the stream ends; `None` when the session is over.
    fn next_frame(&mut self) -> Option<String> {
        loop {
            if self.shared.is_closed() {
                return None;
            }
            match self.event() {
                Ok(data) => {
                    if self.drop_hello {
                        self.drop_hello = false;
                        if serde_json::from_str::<Value>(&data).is_ok_and(|f| f["type"] == "hello")
                        {
                            continue;
                        }
                    }
                    self.shared.note_seen(&data);
                    return Some(data);
                }
                Err(_) if self.shared.is_closed() => return None,
                Err(_) => {
                    if !self.reconnect() {
                        return None;
                    }
                }
            }
        }
    }

    /// A new session in place of one that ended: true when it is open and its rooms are resubscribed.
    fn reconnect(&mut self) -> bool {
        self.shared.release();
        self.body = None;
        for attempt in 0..self.shared.opts.reconnect_attempts {
            if self.shared.is_closed() {
                return false;
            }
            match self.shared.open_stream() {
                Ok(body) => {
                    self.body = Some(body);
                    self.drop_hello = true;
                    self.resubscribe();
                    return true;
                }
                Err(_) => std::thread::sleep(self.shared.opts.reconnect_delay * (attempt + 1)),
            }
        }
        false
    }

    fn resubscribe(&mut self) {
        let subs: Vec<(String, u64)> = self
            .shared
            .lock()
            .subs
            .iter()
            .map(|(r, s)| (r.clone(), *s))
            .collect();
        for (room, seq) in subs {
            self.rid += 1;
            let frame = json!({
                "v": 1, "type": "request", "id": format!("r{}", self.rid),
                "method": "subscribe", "params": {"room": room, "from_seq": seq}
            });
            if self.shared.post_line(&format!("{frame}\n")).is_err() {
                return;
            }
        }
    }
}

impl Read for SseReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.pos >= self.out.len() {
            let Some(frame) = self.next_frame() else {
                return Ok(0);
            };
            self.out = frame.into_bytes();
            self.out.push(b'\n');
            self.pos = 0;
        }
        let n = buf.len().min(self.out.len() - self.pos);
        buf[..n].copy_from_slice(&self.out[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// The requests as a `Write`: each line written is posted to the session.
struct PostWriter {
    shared: Arc<Shared>,
    pending: Vec<u8>,
}

impl Write for PostWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(buf);
        while let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            let line = String::from_utf8(line)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "a frame is UTF-8"))?;
            self.shared.note_sent(&line);
            self.shared.post_line(&line)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Client {
    /// Connect to a `klatch-server http --listen ADDR` at `url` (`http://127.0.0.1:PORT`, loopback only) as `member`
    /// with its credential `token`, the only way to be known over HTTP (a worker credential the human issued); the
    /// member is verified. See [`Client::connect_http_with`] for the deadlines and the reconnect.
    pub fn connect_http(url: &str, member: &str, token: &str) -> Result<Self> {
        Self::connect_http_with(url, member, token, &HttpOptions::new())
    }

    /// [`Client::connect_http`] with `opts`. A refused credential is an `Error::Api` with code `identity.refused` (the
    /// message never repeats the token); a url that is not plain http to a loopback address, or a member or token
    /// that could not be sent as a header value, is an `Error::Session` before anything connects; a server nobody
    /// listens on is an `Error::Session`. If the stream ends without a close it is reopened (up to
    /// `reconnect_attempts` times) and every subscribed room is resubscribed from the last `seq` seen, so messages
    /// resume without a gap or a repeat; when that fails the session is over. [`Client::close`] ends the session and
    /// answers 0.
    pub fn connect_http_with(
        url: &str,
        member: &str,
        token: &str,
        opts: &HttpOptions,
    ) -> Result<Self> {
        let addr = parse_url(url).map_err(Error::Session)?;
        if !header_safe(member) || !header_safe(token) {
            return Err(Error::Session(
                "a member and a token are visible characters without spaces".into(),
            ));
        }
        let shared = Arc::new(Shared {
            addr,
            member: member.into(),
            token: token.into(),
            opts: opts.clone(),
            closed: AtomicBool::new(false),
            state: Mutex::new(State::default()),
        });
        let body = shared.open_stream()?;
        let reader = SseReader {
            shared: shared.clone(),
            body: Some(body),
            out: Vec::new(),
            pos: 0,
            drop_hello: false,
            rid: 0,
        };
        let writer = PostWriter {
            shared: shared.clone(),
            pending: Vec::new(),
        };
        let for_shutdown = shared.clone();
        let shutdown = Box::new(move || for_shutdown.shut());
        match Self::over(reader, writer, shutdown, &Identity::none()) {
            Ok(c) => Ok(c),
            Err(e) => {
                shared.shut();
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_plain_loopback_http_url_with_a_port_is_a_server_address() {
        for (url, want) in [
            ("http://127.0.0.1:7777", "127.0.0.1:7777"),
            ("http://127.0.0.1:7777/", "127.0.0.1:7777"),
            ("http://localhost:80", "127.0.0.1:80"),
            ("http://[::1]:9", "[::1]:9"),
        ] {
            assert_eq!(parse_url(url).unwrap().to_string(), want, "{url}");
        }
        for url in [
            "https://127.0.0.1:1",
            "127.0.0.1:1",
            "http://127.0.0.2:1",
            "http://127.0.0.1",
            "http://127.0.0.1:",
            "http://127.0.0.1:0",
            "http://127.0.0.1:65536",
            "http://127.0.0.1:+1",
            "http://127.0.0.1:1/x",
            "http://a@127.0.0.1:1",
            "http://localhost.evil.test:1",
            "http://[::ffff:127.0.0.1]:1",
            "http://[::1:1",
            "http://0.0.0.0:1",
            "",
        ] {
            assert!(parse_url(url).is_err(), "{url}");
        }
    }

    #[test]
    fn a_header_value_is_visible_ascii_without_spaces() {
        assert!(header_safe("w-1") && header_safe("a1b2.c3_d4"));
        for bad in ["", "a b", "a\r\nb", "a\nb", "a\0b", "é"] {
            assert!(!header_safe(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_line_ends_at_a_newline_and_an_unfinished_line_is_not_one() {
        let mut r = io::Cursor::new(b"data: x\r\n\nrest".to_vec());
        assert_eq!(read_line(&mut r).unwrap().as_deref(), Some("data: x"));
        assert_eq!(read_line(&mut r).unwrap().as_deref(), Some(""));
        assert_eq!(read_line(&mut r).unwrap(), None);
    }
}
