//! Golden bytes of the HTTP wire (`klatch-server http`, #1059; characterisation for the code health ticket #1449). The
//! other HTTP tests check what a client can rely on (status, one header); this one pins the whole answer: the exact
//! bytes of every refusal (status line, header order, error body) and of the event-stream head and its first event, so
//! a refactor of `wire_http` cannot change what goes on the wire without a test failing. The server runs as a child
//! process against a temp store; a test is a client of plain sockets.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

static N: AtomicU32 = AtomicU32::new(0);
/// Issuing a credential sets this process's environment: one test at a time.
static ENV: Mutex<()> = Mutex::new(());

struct Dir(PathBuf);

impl Dir {
    fn new() -> Dir {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = PathBuf::from(format!("/tmp/kg{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["", "h"] {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(dir.join(sub))
                .unwrap();
        }
        Dir(dir)
    }
    fn home(&self) -> PathBuf {
        self.0.join("h")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn issue_credential(dir: &Dir, member: &str) -> String {
    unsafe {
        std::env::set_var("KLATCH_HOME", dir.home());
        std::env::set_var("KLATCH_KEY_DIR", dir.home().join("keys"));
        std::env::remove_var("KLATCH_NAME");
        std::env::remove_var("KLATCH_IDENTITY_FILE");
    }
    let store = klatch::member::custody::open_store(&klatch::member::db_path().unwrap()).unwrap();
    let gateway =
        klatch::gateway::Gateway::new(store, klatch::member::CliClock, klatch::gateway::AllowAll);
    let spec = klatch::gateway::CredentialSpec {
        member,
        clearance: "public",
        ttl: 3600,
        max_life: None,
        parent: None,
    };
    gateway
        .admin_issue_credential(&klatch::domain::Principal::unattributed("launcher"), &spec)
        .unwrap()
        .to_text()
        .to_string()
}

struct Server {
    child: Child,
    addr: SocketAddr,
}

/// The address the readiness line names, once the line is complete (its newline is there).
fn logged_addr(log: &str) -> Option<SocketAddr> {
    let rest = log.split("http listening on ").nth(1)?;
    if !rest.contains('\n') {
        return None;
    }
    Some(
        rest.split_whitespace()
            .next()?
            .parse()
            .expect("the logged address"),
    )
}

impl Server {
    fn started(dir: &Dir) -> Server {
        let log = dir.0.join("err.log");
        let child = Command::new(env!("CARGO_BIN_EXE_klatch-server"))
            .args(["http", "--listen", "127.0.0.1:0"])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", dir.home())
            .env("KLATCH_HOME", dir.home())
            .env("KLATCH_KEY_DIR", dir.home().join("keys"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(std::fs::File::create(&log).unwrap()))
            .spawn()
            .expect("start klatch-server");
        let mut server = Server {
            child,
            addr: "127.0.0.1:0".parse().unwrap(),
        };
        let until = Instant::now() + Duration::from_secs(60);
        while Instant::now() < until {
            let text = std::fs::read_to_string(&log).unwrap_or_default();
            if let Some(addr) = logged_addr(&text) {
                server.addr = addr;
                return server;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!(
            "the server never listened:\n{text}",
            text = std::fs::read_to_string(&log).unwrap_or_default()
        );
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        unsafe {
            libc::kill(self.child.id() as libc::pid_t, libc::SIGKILL);
        }
        let _ = self.child.wait();
    }
}

fn connect(addr: SocketAddr) -> TcpStream {
    let s = TcpStream::connect(addr).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    s
}

/// Send raw request bytes and read the whole answer (the server closes after one exchange).
fn raw(addr: SocketAddr, request: &[u8]) -> Vec<u8> {
    let mut s = connect(addr);
    let _ = s.write_all(request);
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    out
}

fn text(addr: SocketAddr, request: &str) -> String {
    String::from_utf8(raw(addr, request.as_bytes())).expect("an answer in text")
}

/// The one shape of every plain answer of the listener: the golden form, written out once.
fn plain(status_line: &str, body: &str, allow: Option<&str>) -> String {
    let allow = allow.map(|a| format!("Allow: {a}\r\n")).unwrap_or_default();
    format!(
        "HTTP/1.1 {status_line}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n{allow}\r\n{body}",
        body.len()
    )
}

/// Requests that are refused before any credential matters, and the exact answer to each.
fn refusal_cases() -> Vec<(&'static str, String)> {
    vec![
        (
            "GET /nope HTTP/1.1\r\nHost: x\r\n\r\n",
            plain("404 Not Found", "not found", None),
        ),
        (
            "POST /v1/stream HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\n\r\n",
            plain("405 Method Not Allowed", "method not allowed", Some("GET")),
        ),
        (
            "GET /v1/frames HTTP/1.1\r\nHost: x\r\n\r\n",
            plain("405 Method Not Allowed", "method not allowed", Some("POST")),
        ),
        (
            "GET /mcp HTTP/1.1\r\nHost: x\r\n\r\n",
            plain("405 Method Not Allowed", "method not allowed", Some("POST")),
        ),
        (
            "GET /v1/stream HTTP/1.1\r\nHost: x\r\n\r\n",
            plain("401 Unauthorized", "unauthorized", None),
        ),
        (
            "GET /v1/stream HTTP/1.1\r\nHost: x\r\nX-Klatch-Member: w\r\nAuthorization: Bearer nope\r\n\r\n",
            plain("401 Unauthorized", "unauthorized", None),
        ),
        (
            "POST /v1/frames HTTP/1.1\r\nHost: x\r\nContent-Length: 1\r\n\r\nx",
            plain("401 Unauthorized", "unauthorized", None),
        ),
        (
            "GARBAGE\r\n\r\n",
            plain("400 Bad Request", "bad request line", None),
        ),
        (
            "GET /v1/stream HTTP/2\r\n\r\n",
            plain("400 Bad Request", "unsupported protocol version", None),
        ),
        (
            "GET /v1/stream HTTP/1.1\r\nno colon here\r\n\r\n",
            plain("400 Bad Request", "bad header line", None),
        ),
        (
            "POST /v1/frames HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\n",
            plain("400 Bad Request", "duplicate content-length", None),
        ),
        (
            "POST /v1/frames HTTP/1.1\r\nContent-Length: abc\r\n\r\n",
            plain("400 Bad Request", "bad content-length", None),
        ),
        (
            "POST /v1/frames HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
            plain(
                "501 Not Implemented",
                "transfer-encoding is not supported",
                None,
            ),
        ),
    ]
}

#[test]
fn every_refusal_before_a_credential_is_the_same_golden_bytes() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let s = Server::started(&dir);
    for (request, expected) in refusal_cases() {
        assert_eq!(text(s.addr, request), expected, "for {request:?}");
    }
    // a head over 16 KiB
    let long = format!(
        "GET /v1/stream HTTP/1.1\r\nX-Pad: {}\r\n\r\n",
        "a".repeat(17 * 1024)
    );
    assert_eq!(
        text(s.addr, &long),
        plain(
            "431 Request Header Fields Too Large",
            "headers too large",
            None
        )
    );
}

/// Read the head of an event stream and its first event: (head text, the first event's text).
fn stream_head_and_first_event(s: &mut TcpStream) -> (String, String) {
    let mut seen = Vec::new();
    let mut byte = [0u8; 1];
    let (mut head_end, mut event_end) = (None, None);
    while event_end.is_none() {
        s.read_exact(&mut byte).expect("the stream is open");
        seen.push(byte[0]);
        if head_end.is_none() && seen.ends_with(b"\r\n\r\n") {
            head_end = Some(seen.len());
        } else if head_end.is_some() && seen.ends_with(b"\n\n") {
            event_end = Some(seen.len());
        }
    }
    let text = String::from_utf8(seen).unwrap();
    let (head, event) = text.split_at(head_end.unwrap());
    (head.to_string(), event.to_string())
}

fn session_id_of(head: &str) -> String {
    head.lines()
        .find_map(|l| l.strip_prefix("X-Klatch-Session: "))
        .expect("the stream names its session")
        .to_string()
}

#[test]
fn the_stream_head_and_first_event_and_the_post_answers_are_golden_bytes() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let token = issue_credential(&dir, "w-1");
    let other = issue_credential(&dir, "w-2");
    let s = Server::started(&dir);
    let auth = format!("X-Klatch-Member: w-1\r\nAuthorization: Bearer {token}\r\n");
    let mut stream = connect(s.addr);
    stream
        .write_all(format!("GET /v1/stream HTTP/1.1\r\nHost: x\r\n{auth}\r\n").as_bytes())
        .unwrap();
    let (head, first) = stream_head_and_first_event(&mut stream);
    let id = session_id_of(&head);
    assert_eq!(id.len(), 32);
    assert_eq!(
        head,
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: close\r\nX-Accel-Buffering: no\r\nX-Klatch-Session: {id}\r\n\r\n"
        )
    );
    assert!(first.starts_with("event: hello\ndata: {"), "{first}");
    assert!(first.ends_with("}\n\n"), "{first}");
    assert!(!first.contains("\nid:"), "{first}");

    let session = format!("X-Klatch-Session: {id}\r\n");
    let post = |headers: &str, body: &str| {
        format!(
            "POST /v1/frames HTTP/1.1\r\nHost: x\r\n{auth}{headers}Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
    };
    let unknown = format!("X-Klatch-Session: {}\r\n", "f".repeat(32));
    let no_such = plain("404 Not Found", "no such session", None);
    let cases = [
        // no session header; a session that never was; a malformed id; an empty body
        (
            post("", "{}"),
            plain("400 Bad Request", "x-klatch-session is required", None),
        ),
        (post(&unknown, "{}"), no_such.clone()),
        (post("X-Klatch-Session: short\r\n", "{}"), no_such.clone()),
        (
            post(&session, ""),
            plain("400 Bad Request", "an empty body carries no frame", None),
        ),
        // a session of another member is as absent as one that never was
        (
            format!(
                "POST /v1/frames HTTP/1.1\r\nX-Klatch-Member: w-2\r\nAuthorization: Bearer {other}\r\n{session}Content-Length: 2\r\n\r\n{{}}"
            ),
            no_such,
        ),
        // a body over the limit is refused before the session is looked up
        (
            format!(
                "POST /v1/frames HTTP/1.1\r\n{auth}{session}Content-Length: {}\r\n\r\n",
                1024 * 1024 + 1
            ),
            plain("413 Content Too Large", "body too large", None),
        ),
        // no Content-Length on a POST
        (
            format!("POST /v1/frames HTTP/1.1\r\n{auth}{session}\r\n"),
            plain("411 Length Required", "content-length required", None),
        ),
    ];
    for (request, expected) in cases {
        assert_eq!(text(s.addr, &request), expected, "for {request:?}");
    }
    // a frame is accepted with nothing but 202, and its answer arrives on the stream as an event
    let ping = r#"{"v":1,"type":"request","id":"r1","method":"ping","params":{}}"#;
    assert_eq!(
        text(s.addr, &post(&session, ping)),
        plain("202 Accepted", "accepted", None)
    );
    let seen = read_until_response(&mut stream);
    assert!(seen.contains("event: response\ndata: {"), "{seen}");
    assert!(seen.ends_with("\n\n"), "{seen}");
}

/// Read the stream until a `response` event has arrived (or 20 seconds pass).
fn read_until_response(stream: &mut TcpStream) -> String {
    let mut seen = String::new();
    let mut buf = [0u8; 4096];
    let until = Instant::now() + Duration::from_secs(20);
    while !seen.contains("event: response\n") && Instant::now() < until {
        let n = stream.read(&mut buf).expect("the stream stays open");
        seen.push_str(&String::from_utf8_lossy(&buf[..n]));
    }
    seen
}
