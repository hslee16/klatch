//! `klatch-server http --listen 127.0.0.1:PORT` (#1059, phase 3 of epic #1056): the klatch wire protocol over HTTP and
//! server-sent events, loopback only. The server runs as a child process against a temp store; a test is a client of
//! plain sockets. Covered: a non-loopback bind is refused before anything listens; every route needs a credential (a
//! bearer token and the member it was issued to, checked live); `GET /v1/stream` gives one session as server-sent
//! events (`hello` first, then every frame of the session protocol, as the same JSON line the stdio session writes) and
//! `POST /v1/frames` writes frames into it; two members share one gateway with dense sequence numbers; a reconnect
//! resumes from the same sequence numbers; limits; SIGTERM; and a conformance run: the scenario of the daemon's test
//! gives the same frames through the in-process session loop and through HTTP and SSE.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

static N: AtomicU32 = AtomicU32::new(0);
/// The library reads its environment (credential setup and the conformance run open a store in this process): one test
/// at a time may set it.
static ENV: Mutex<()> = Mutex::new(());

/// A short directory under /tmp with a store home in it.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Dir {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = PathBuf::from(format!("/tmp/kh{}-{n}", std::process::id()));
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
        if std::thread::panicking() {
            eprintln!(
                "server log of {}:\n{}",
                self.0.display(),
                std::fs::read_to_string(self.0.join("err.log")).unwrap_or_default()
            );
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Issue a credential for `member` in the store of `dir` (this process reads the environment: the caller holds ENV).
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
    let token = gateway
        .admin_issue_credential(&klatch::domain::Principal::unattributed("launcher"), &spec)
        .unwrap();
    token.to_text().to_string()
}

struct Server {
    child: Child,
    addr: SocketAddr,
    log: PathBuf,
}

impl Server {
    fn start(dir: &Dir, args: &[&str]) -> Child {
        Command::new(env!("CARGO_BIN_EXE_klatch-server"))
            .arg("http")
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", dir.home())
            .env("KLATCH_HOME", dir.home())
            .env("KLATCH_KEY_DIR", dir.home().join("keys"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(dir.0.join("err.log"))
                    .unwrap(),
            ))
            .spawn()
            .expect("start klatch-server")
    }

    /// Start on a port the system chooses and wait until the log names it.
    fn started(dir: &Dir, extra: &[&str]) -> Server {
        let mut args = vec!["--listen", "127.0.0.1:0"];
        args.extend_from_slice(extra);
        let mut server = Server {
            child: Self::start(dir, &args),
            addr: "127.0.0.1:0".parse().unwrap(),
            log: dir.0.join("err.log"),
        };
        let until = Instant::now() + Duration::from_secs(60);
        while Instant::now() < until {
            let text = std::fs::read_to_string(&server.log).unwrap_or_default();
            // Trust the line only once it is complete (its newline is there), never half an address (#1173, #1207).
            if let Some(rest) = text.split("http listening on ").nth(1)
                && rest.contains('\n')
                && let Some(addr) = rest.split_whitespace().next()
            {
                server.addr = addr.parse().expect("the logged address");
                return server;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!(
            "the server never listened; its stderr was:\n{}",
            std::fs::read_to_string(&server.log).unwrap_or_default()
        );
    }

    fn signal(&self, sig: libc::c_int) {
        unsafe {
            libc::kill(self.child.id() as libc::pid_t, sig);
        }
    }

    fn finish(mut self) -> (std::process::ExitStatus, String) {
        let until = Instant::now() + Duration::from_secs(20);
        let status = loop {
            if let Some(s) = self.child.try_wait().unwrap() {
                break s;
            }
            assert!(Instant::now() < until, "the server did not stop");
            std::thread::sleep(Duration::from_millis(20));
        };
        (
            status,
            std::fs::read_to_string(&self.log).unwrap_or_default(),
        )
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

/// One whole HTTP exchange: status, headers (names lower-cased) and body.
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

fn send_request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> TcpStream {
    let mut s = TcpStream::connect(addr).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\n");
    for (n, v) in headers {
        head.push_str(&format!("{n}: {v}\r\n"));
    }
    if method == "POST" {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    // the server may answer and close before it has read everything (a refusal): a failed write is not a failure
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
    s
}

fn read_head(r: &mut BufReader<TcpStream>) -> (u16, Vec<(String, String)>) {
    let mut line = String::new();
    r.read_line(&mut line).expect("a status line");
    let status: u16 = line
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("status line {line:?}"));
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        r.read_line(&mut h).unwrap();
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        let (n, v) = h.split_once(':').unwrap();
        headers.push((n.to_ascii_lowercase(), v.trim().to_string()));
    }
    (status, headers)
}

fn exchange(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Reply {
    let s = send_request(addr, method, path, headers, body);
    let mut r = BufReader::new(s);
    let (status, headers) = read_head(&mut r);
    let mut body = String::new();
    let _ = r.read_to_string(&mut body);
    Reply {
        status,
        headers,
        body,
    }
}

/// Only the status of an exchange: the connection is dropped after the head. For a request whose answer may be an event
/// stream, which never ends (heartbeats keep resetting a read timeout, so reading it to the end would wait forever).
fn status_of(addr: SocketAddr, path: &str, headers: &[(&str, &str)]) -> u16 {
    let s = send_request(addr, "GET", path, headers, b"");
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    read_head(&mut BufReader::new(s)).0
}

fn auth<'a>(member: &'a str, token: &'a str, bearer: &'a str) -> [(&'a str, &'a str); 2] {
    let _ = (member, token);
    [("X-Klatch-Member", member), ("Authorization", bearer)]
}

/// One server-sent-events session of a member.
struct Sse {
    addr: SocketAddr,
    member: String,
    bearer: String,
    session: String,
    r: BufReader<TcpStream>,
    /// Frames that arrived while a response was awaited, with the `id:` line each came with.
    extra: Vec<(Option<String>, Value)>,
    next: u32,
}

impl Sse {
    fn open(addr: SocketAddr, member: &str, token: &str) -> Sse {
        let bearer = format!("Bearer {token}");
        let s = send_request(
            addr,
            "GET",
            "/v1/stream",
            &auth(member, token, &bearer),
            b"",
        );
        let mut r = BufReader::new(s);
        let (status, headers) = read_head(&mut r);
        assert_eq!(status, 200, "{headers:?}");
        let get = |n: &str| {
            headers
                .iter()
                .find(|(h, _)| h == n)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        assert!(get("content-type").starts_with("text/event-stream"));
        assert_eq!(get("cache-control"), "no-store");
        let session = get("x-klatch-session");
        assert!(session.len() >= 32, "{session:?}");
        Sse {
            addr,
            member: member.into(),
            bearer,
            session,
            r,
            extra: Vec::new(),
            next: 0,
        }
    }

    /// The next event: its `id:` line (when it has one) and the frame in its `data:` line. Comment lines are skipped.
    fn recv(&mut self) -> Option<(Option<String>, Value)> {
        let mut id = None;
        let mut data: Option<Value> = None;
        loop {
            let mut line = String::new();
            match self.r.read_line(&mut line) {
                Ok(0) => return None,
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::TimedOut
                            | std::io::ErrorKind::ConnectionReset
                    ) =>
                {
                    return None;
                }
                Err(e) => panic!("read: {e}"),
            }
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if let Some(d) = data {
                    return Some((id, d));
                }
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            if let Some(v) = line.strip_prefix("id: ") {
                id = Some(v.to_string());
            } else if let Some(v) = line.strip_prefix("data: ") {
                assert!(data.is_none(), "one data line per event");
                data = Some(serde_json::from_str(v).unwrap_or_else(|e| panic!("{v:?}: {e}")));
            } else if line.starts_with("event: ") {
                // the event name is the frame's type: checked where a test cares
            } else {
                panic!("unexpected line in the stream: {line:?}");
            }
        }
    }

    fn frame(&mut self) -> Value {
        self.recv().expect("a frame").1
    }

    /// POST one frame (a JSON line) to this session; the status.
    fn post_frame(&self, frame: &Value) -> u16 {
        let mut body = frame.to_string().into_bytes();
        body.push(b'\n');
        let mut h = auth(&self.member, "", &self.bearer).to_vec();
        h.push(("X-Klatch-Session", &self.session));
        exchange(self.addr, "POST", "/v1/frames", &h, &body).status
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = format!("r{}", self.next);
        let status = self
            .post_frame(&json!({"v":1,"type":"request","id":id,"method":method,"params":params}));
        assert_eq!(status, 202);
        loop {
            let (eid, f) = self
                .recv()
                .unwrap_or_else(|| panic!("closed before the response to {method}"));
            if f["type"] == "response" && f["id"] == id.as_str() {
                return f;
            }
            self.extra.push((eid, f));
        }
    }

    fn post(&mut self, room: &str, body: &str) -> i64 {
        let r = self.request("post", json!({"room": room, "body": body}));
        assert_eq!(r["ok"], true, "{r}");
        r["result"]["seq"].as_i64().unwrap()
    }
}

// ---------------------------------------------------------------------------------------------------------------

#[test]
fn a_non_loopback_bind_is_refused_before_anything_listens() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    for addr in [
        "0.0.0.0:0",
        "[::]:0",
        "192.168.1.5:0",
        "127.0.0.2:0",
        "localhost:0",
        "10.0.0.1:9",
        "[::ffff:127.0.0.1]:0",
    ] {
        let dir = Dir::new();
        let mut child = Server::start(&dir, &["--listen", addr]);
        let status = child.wait().unwrap();
        let err = std::fs::read_to_string(dir.0.join("err.log")).unwrap_or_default();
        assert_eq!(status.code(), Some(1), "{addr}: {err}");
        assert!(
            err.contains("refusing to listen") && err.contains("#157") && err.contains("#161"),
            "{addr}: {err}"
        );
        assert!(!err.contains("http listening"), "{addr}: {err}");
    }
    // the listen address has no default and the usage is refused, not guessed
    let dir = Dir::new();
    let status = Server::start(&dir, &[]).wait().unwrap();
    assert_eq!(status.code(), Some(1));
}

#[test]
fn every_route_needs_a_live_credential_for_the_member_named() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let token = issue_credential(&dir, "w-1");
    let _other = issue_credential(&dir, "w-2");
    let s = Server::started(&dir, &[]);
    let good = format!("Bearer {token}");
    let wrong = format!("Bearer {}", "0".repeat(64));
    let cases: Vec<Vec<(&str, &str)>> = vec![
        vec![],
        vec![("X-Klatch-Member", "w-1")],
        vec![("Authorization", good.as_str())],
        vec![
            ("X-Klatch-Member", "w-1"),
            ("Authorization", wrong.as_str()),
        ],
        // another member's name with this token
        vec![("X-Klatch-Member", "w-2"), ("Authorization", good.as_str())],
        vec![("X-Klatch-Member", "w-1"), ("Authorization", "Bearer nope")],
        vec![("X-Klatch-Member", "w-1"), ("Authorization", "Basic abc")],
        // a typed name is no credential here: there is no unauthenticated floor over HTTP
        vec![("X-Klatch-Member", "alice")],
    ];
    for headers in &cases {
        for (method, path) in [("GET", "/v1/stream"), ("POST", "/v1/frames")] {
            let mut h = headers.clone();
            h.push(("X-Klatch-Session", "0123456789abcdef0123456789abcdef"));
            let r = exchange(s.addr, method, path, &h, b"{}\n");
            assert_eq!(r.status, 401, "{method} {path} {headers:?}");
            assert_eq!(r.body, "unauthorized");
            assert!(!r.body.contains(&token) && !r.headers.iter().any(|(_, v)| v.contains(&token)));
        }
    }
    // the token is never in the server's log
    let (_status, log) = {
        s.signal(libc::SIGTERM);
        s.finish()
    };
    assert!(!log.contains(&token), "{log}");
}

/// Behind a reverse proxy or stunnel the proxy's word about the caller is only a header (#110, #161): the address, a
/// forwarded user, a client-certificate header and a loopback source (the proxy is on this host) give no identity and
/// change none. The credential alone decides.
#[test]
fn forwarded_identity_headers_give_no_identity_and_change_none() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let token = issue_credential(&dir, "w-1");
    let _other = issue_credential(&dir, "w-2");
    let s = Server::started(&dir, &[]);
    let forged: &[(&str, &str)] = &[
        ("X-Forwarded-For", "10.0.0.7"),
        ("X-Real-IP", "10.0.0.7"),
        ("Forwarded", "for=10.0.0.7;proto=https"),
        ("X-Forwarded-User", "w-1"),
        ("X-Remote-User", "w-1"),
        ("X-Klatch-Forwarded-Member", "w-1"),
        ("X-SSL-Client-S-DN", "CN=w-1"),
        ("X-Client-Cert", "CN=w-1"),
        ("X-Client-Verify", "SUCCESS"),
    ];
    // no credential: every forged header together is still a 401, on every route, from a loopback source
    for (method, path) in [("GET", "/v1/stream"), ("POST", "/v1/frames")] {
        let mut h = forged.to_vec();
        h.push(("X-Klatch-Session", "0123456789abcdef0123456789abcdef"));
        let r = exchange(s.addr, method, path, &h, b"{}\n");
        assert_eq!(r.status, 401, "{method} {path}");
        let mut h = forged.to_vec();
        h.push(("X-Klatch-Member", "w-1"));
        let r = exchange(s.addr, method, path, &h, b"{}\n");
        assert_eq!(r.status, 401, "{method} {path} with a member name only");
    }
    let r = exchange(s.addr, "POST", "/mcp", forged, b"{}");
    assert_eq!(r.status, 401, "/mcp");
    // a credential for w-1 with headers that claim w-2: the session is w-1's, whatever the proxy says
    let bearer = format!("Bearer {token}");
    let mut h: Vec<(&str, &str)> = forged
        .iter()
        .map(|(n, v)| (*n, if v == &"w-1" { "w-2" } else { *v }))
        .collect();
    h.push(("X-Klatch-Member", "w-1"));
    h.push(("Authorization", bearer.as_str()));
    let st = send_request(s.addr, "GET", "/v1/stream", &h, b"");
    let mut r = BufReader::new(st);
    let (status, _) = read_head(&mut r);
    assert_eq!(status, 200);
    let mut line = String::new();
    let mut hello = String::new();
    while !hello.contains("\"type\":\"hello\"") {
        line.clear();
        r.read_line(&mut line).unwrap();
        hello.push_str(&line);
        assert!(!line.is_empty(), "no hello: {hello}");
    }
    assert!(hello.contains("\"member\":\"w-1\""), "{hello}");
    assert!(!hello.contains("w-2"), "{hello}");
}

#[test]
fn the_stream_begins_with_a_verified_hello_and_carries_frames_as_the_session_protocol_writes_them()
{
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let token = issue_credential(&dir, "w-1");
    let s = Server::started(&dir, &[]);
    let mut c = Sse::open(s.addr, "w-1", &token);
    let (id, hello) = c.recv().unwrap();
    assert_eq!(id, None);
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["member"], "w-1");
    assert_eq!(hello["attribution"], "verified");
    let r = c.request("join", json!({"room": "r"}));
    assert_eq!(r["ok"], true, "{r}");
    c.post("r", "one");
    c.post("r", "two");
    let r = c.request("subscribe", json!({"room": "r", "from_seq": 0}));
    assert_eq!(r["ok"], true, "{r}");
    // each message event carries its room and sequence number as the event id; the data is the frame itself
    let mut seen = Vec::new();
    while seen.len() < 3 {
        let (eid, f) = c.recv().expect("stream frames");
        match f["type"].as_str().unwrap() {
            "message" => {
                assert_eq!(eid.as_deref(), Some(format!("r:{}", f["seq"]).as_str()));
                assert_eq!(f["attribution"], "verified");
                seen.push(f["seq"].as_i64().unwrap());
            }
            "caught_up" => seen.push(-1),
            other => panic!("{other}: {f}"),
        }
    }
    seen.sort();
    assert_eq!(seen, [-1, 1, 2]);
}

#[test]
fn a_frame_posted_to_no_session_or_to_another_members_session_is_not_found() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let t1 = issue_credential(&dir, "w-1");
    let t2 = issue_credential(&dir, "w-2");
    let s = Server::started(&dir, &[]);
    let one = Sse::open(s.addr, "w-1", &t1);
    let b1 = format!("Bearer {t1}");
    let b2 = format!("Bearer {t2}");
    let line = b"{\"v\":1,\"type\":\"request\",\"id\":\"x\",\"method\":\"whoami\",\"params\":{}}\n";
    for (member, bearer, session) in [
        ("w-1", b1.as_str(), "ffffffffffffffffffffffffffffffff"),
        ("w-1", b1.as_str(), "not a session"),
        ("w-2", b2.as_str(), one.session.as_str()),
    ] {
        let r = exchange(
            s.addr,
            "POST",
            "/v1/frames",
            &[
                ("X-Klatch-Member", member),
                ("Authorization", bearer),
                ("X-Klatch-Session", session),
            ],
            line,
        );
        assert_eq!(r.status, 404, "{member} {session}");
    }
    // no session header at all is a bad request, not a guess
    let r = exchange(
        s.addr,
        "POST",
        "/v1/frames",
        &[("X-Klatch-Member", "w-1"), ("Authorization", b1.as_str())],
        line,
    );
    assert_eq!(r.status, 400);
}

#[test]
fn two_members_share_one_gateway_with_dense_sequence_numbers() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let ta = issue_credential(&dir, "alice");
    let tb = issue_credential(&dir, "bob");
    let s = Server::started(&dir, &[]);
    let mut a = Sse::open(s.addr, "alice", &ta);
    let mut b = Sse::open(s.addr, "bob", &tb);
    a.frame();
    b.frame();
    a.request("join", json!({"room": "lobby"}));
    b.request("join", json!({"room": "lobby"}));
    let mut seqs = Vec::new();
    for i in 0..5 {
        seqs.push(a.post("lobby", &format!("a{i}")));
        seqs.push(b.post("lobby", &format!("b{i}")));
    }
    assert_eq!(seqs, (1..=10).collect::<Vec<_>>());
    // a third session reads all ten, in order, from the stream
    let mut c = Sse::open(s.addr, "alice", &ta);
    c.frame();
    c.request("subscribe", json!({"room": "lobby", "from_seq": 0}));
    let mut got = Vec::new();
    while got.len() < 10 {
        let (_, f) = c.recv().expect("frames");
        if f["type"] == "message" {
            got.push((
                f["seq"].as_i64().unwrap(),
                f["sender"].as_str().unwrap().to_string(),
            ));
        }
    }
    assert_eq!(
        got.iter().map(|g| g.0).collect::<Vec<_>>(),
        (1..=10).collect::<Vec<_>>()
    );
    assert_eq!(got[0].1, "alice");
    assert_eq!(got[1].1, "bob");
}

#[test]
fn a_new_stream_resumes_from_the_sequence_numbers_a_client_last_saw() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let ta = issue_credential(&dir, "alice");
    let tb = issue_credential(&dir, "bob");
    let s = Server::started(&dir, &[]);
    let mut a = Sse::open(s.addr, "alice", &ta);
    a.frame();
    a.request("join", json!({"room": "r"}));
    a.request("subscribe", json!({"room": "r", "from_seq": 0}));
    let mut b = Sse::open(s.addr, "bob", &tb);
    b.frame();
    b.request("join", json!({"room": "r"}));
    for i in 0..3 {
        b.post("r", &format!("m{i}"));
    }
    let mut last = 0;
    while last < 3 {
        let (_, f) = a.recv().expect("live frames");
        if f["type"] == "message" {
            last = f["seq"].as_i64().unwrap();
        }
    }
    // alice's connection drops; the session ends, and a post to it is not found
    let old = a.session.clone();
    drop(a);
    b.post("r", "while-away-1");
    b.post("r", "while-away-2");
    let mut a2 = Sse::open(s.addr, "alice", &ta);
    assert_ne!(a2.session, old);
    a2.frame();
    let line = b"{\"v\":1,\"type\":\"request\",\"id\":\"x\",\"method\":\"whoami\",\"params\":{}}\n";
    let bearer = format!("Bearer {ta}");
    let gone = {
        // the old session may take a moment to end: the answer settles to 404
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            let r = exchange(
                s.addr,
                "POST",
                "/v1/frames",
                &[
                    ("X-Klatch-Member", "alice"),
                    ("Authorization", bearer.as_str()),
                    ("X-Klatch-Session", old.as_str()),
                ],
                line,
            );
            if r.status == 404 || Instant::now() > until {
                break r.status;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    assert_eq!(gone, 404);
    // resume: from the last sequence number seen, exactly the missed messages arrive, with their original numbers
    a2.request("subscribe", json!({"room": "r", "from_seq": last}));
    let mut got = Vec::new();
    while got.len() < 2 {
        let (eid, f) = a2.recv().expect("frames");
        if f["type"] == "message" {
            assert_eq!(eid.as_deref(), Some(format!("r:{}", f["seq"]).as_str()));
            got.push((
                f["seq"].as_i64().unwrap(),
                f["body"].as_str().unwrap().to_string(),
            ));
        }
    }
    assert_eq!(
        got,
        [
            (4, "while-away-1".to_string()),
            (5, "while-away-2".to_string())
        ]
    );
}

#[test]
fn garbage_in_a_posted_frame_gets_an_error_frame_on_the_stream_and_the_session_goes_on() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let t = issue_credential(&dir, "w-1");
    let s = Server::started(&dir, &[]);
    let mut c = Sse::open(s.addr, "w-1", &t);
    c.frame();
    let bearer = format!("Bearer {t}");
    let mut h = auth("w-1", "", &bearer).to_vec();
    h.push(("X-Klatch-Session", &c.session));
    let r = exchange(s.addr, "POST", "/v1/frames", &h, b"this is not json\n");
    assert_eq!(r.status, 202);
    let f = c.frame();
    assert_eq!(f["type"], "refusal", "{f}");
    c.request("join", json!({"room": "r"}));
    c.post("r", "still working");
}

#[test]
fn routes_methods_and_sizes_are_limited() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let t = issue_credential(&dir, "w-1");
    let s = Server::started(&dir, &[]);
    let r = exchange(s.addr, "GET", "/nope", &[], b"");
    assert_eq!(r.status, 404);
    let r = exchange(s.addr, "POST", "/v1/stream", &[], b"x");
    assert_eq!(r.status, 405);
    assert_eq!(r.header("allow"), Some("GET"));
    let r = exchange(s.addr, "GET", "/v1/frames", &[], b"");
    assert_eq!(r.status, 405);
    assert_eq!(r.header("allow"), Some("POST"));
    // a body over 1 MiB is refused before it is read, and without the credential check telling anything first
    let big = vec![b'x'; 1024 * 1024 + 1];
    let bearer = format!("Bearer {t}");
    let r = exchange(
        s.addr,
        "POST",
        "/v1/frames",
        &[
            ("X-Klatch-Member", "w-1"),
            ("Authorization", bearer.as_str()),
            ("X-Klatch-Session", "ffffffffffffffffffffffffffffffff"),
        ],
        &big,
    );
    assert_eq!(r.status, 413);
    // chunked bodies are not read
    let mut sock = TcpStream::connect(s.addr).unwrap();
    sock.set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    sock.write_all(
        b"POST /v1/frames HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
    )
    .unwrap();
    let mut text = String::new();
    let _ = sock.read_to_string(&mut text);
    assert!(text.starts_with("HTTP/1.1 501"), "{text}");
}

#[test]
fn connections_over_the_cap_are_refused_and_a_closed_one_frees_its_slot() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let t = issue_credential(&dir, "w-1");
    let s = Server::started(&dir, &["--max-connections", "2"]);
    let mut a = Sse::open(s.addr, "w-1", &t);
    let mut b = Sse::open(s.addr, "w-1", &t);
    a.frame();
    b.frame();
    let bearer = format!("Bearer {t}");
    let h = [
        ("X-Klatch-Member", "w-1"),
        ("Authorization", bearer.as_str()),
    ];
    assert_eq!(status_of(s.addr, "/v1/stream", &h), 503);
    drop(a);
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        let status = status_of(s.addr, "/v1/stream", &h);
        if status != 503 || Instant::now() > until {
            assert_ne!(status, 503, "the slot of the closed connection is freed");
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(b);
}

#[test]
fn sigterm_closes_open_streams_and_stops_the_server() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let t = issue_credential(&dir, "w-1");
    let s = Server::started(&dir, &[]);
    let mut c = Sse::open(s.addr, "w-1", &t);
    c.frame();
    s.signal(libc::SIGTERM);
    // the stream ends
    while c.recv().is_some() {}
    let (status, err) = s.finish();
    assert_eq!(status.code(), Some(0), "{err}");
    assert!(err.contains("http stopped"), "{err}");
}

#[test]
fn a_store_that_cannot_be_opened_stops_the_server_before_it_listens() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    std::fs::write(dir.home().join("rooms.db"), vec![7u8; 4096]).unwrap();
    let status = Server::start(&dir, &["--listen", "127.0.0.1:0"])
        .wait()
        .unwrap();
    let err = std::fs::read_to_string(dir.0.join("err.log")).unwrap_or_default();
    assert_eq!(status.code(), Some(1), "{err}");
    assert!(!err.contains("http listening"), "{err}");
}

// ---------------------------------------------------------------------------------------------------------------
// conformance: the scenario of tests/daemon.rs, through the session loop and through HTTP and SSE

fn scenario(member: &str) -> Vec<Value> {
    let req = |id: &str, method: &str, params: Value| json!({"v":1,"type":"request","id":id,"method":method,"params":params});
    let mut s = vec![req("j", "join", json!({"room": "conf"}))];
    for i in 0..3 {
        s.push(req(
            &format!("p{i}"),
            "post",
            json!({"room": "conf", "body": format!("{member}-{i}")}),
        ));
    }
    s.push(req(
        "post-handoff",
        "post",
        json!({"room": "conf", "body": "h", "kind": "review", "payload": "{\"n\":1}"}),
    ));
    s.push(req("head", "history", json!({"room": "conf", "last": 10})));
    s.push(req(
        "sub",
        "subscribe",
        json!({"room": "conf", "from_seq": 0}),
    ));
    s.push(json!({"v":1,"type":"ack","room":"conf","seq":2}));
    s.push(req("read", "read", json!({"room": "conf"})));
    s.push(req(
        "who",
        "claim",
        json!({"room": "conf", "path": format!("{member}.rs")}),
    ));
    s.push(req("bad", "no_such_method", json!({})));
    s
}

/// Drop what differs between two runs of the same thing: times and random ids; keep every sequence number, sender,
/// attribution, body and error code.
fn normal(mut v: Value, top: bool) -> Value {
    match &mut v {
        Value::Object(m) => {
            m.remove("ts");
            m.remove("last_seen");
            m.remove("expires");
            if !top || m.get("type").is_some_and(|t| t != "response") {
                m.remove("id");
            }
            for (_, x) in m.iter_mut() {
                *x = normal(x.take(), false);
            }
        }
        Value::Array(a) => {
            for x in a.iter_mut() {
                *x = normal(x.take(), false);
            }
        }
        _ => {}
    }
    v
}

/// The scenario through the session loop in this process, as `klatch serve --stdio` runs it, for the same two verified
/// members in turn: the frames after the hello.
fn direct_run(dir: &Dir) -> (Vec<Value>, Vec<Value>) {
    unsafe {
        std::env::set_var("KLATCH_HOME", dir.home());
        std::env::set_var("KLATCH_KEY_DIR", dir.home().join("keys"));
        std::env::remove_var("KLATCH_NAME");
        std::env::remove_var("KLATCH_IDENTITY_FILE");
    }
    let mut runs = Vec::new();
    for name in ["alice", "bob"] {
        let who = klatch::member::Who {
            as_member: Some(name.into()),
            ..Default::default()
        };
        let m = klatch::member::open_member(&who, klatch::member::Call::default())
            .unwrap()
            .unwrap();
        let verified = klatch::domain::Principal::verified(name);
        let input: String = scenario(name).iter().map(|f| format!("{f}\n")).collect();
        let config = klatch::session::SessionConfig::default();
        let mut out = Vec::new();
        klatch::session::Session::new(&m.gateway, &verified, &config)
            .serve(std::io::Cursor::new(input.into_bytes()), &mut out)
            .unwrap();
        let frames: Vec<Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(frames[0]["type"], "hello");
        assert_eq!(frames[0]["member"], name);
        runs.push(
            frames[1..]
                .iter()
                .cloned()
                .map(|f| normal(f, true))
                .collect(),
        );
    }
    (runs.remove(0), runs.remove(0))
}

fn http_run(dir: &Dir, tokens: [&str; 2]) -> (Vec<Value>, Vec<Value>) {
    let s = Server::started(dir, &[]);
    let mut runs = Vec::new();
    for (name, token) in ["alice", "bob"].into_iter().zip(tokens) {
        let mut c = Sse::open(s.addr, name, token);
        let hello = c.frame();
        assert_eq!(hello["member"], name);
        assert_eq!(hello["attribution"], "verified");
        for f in scenario(name) {
            assert_eq!(c.post_frame(&f), 202);
        }
        // read until the last response, then until the stream is quiet
        let mut frames = Vec::new();
        let mut answered = false;
        c.r.get_ref()
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        while !answered {
            let f = c.frame();
            answered = f["type"] == "response" && f["id"] == "bad";
            frames.push(f);
        }
        c.r.get_ref()
            .set_read_timeout(Some(Duration::from_millis(700)))
            .unwrap();
        while let Some((_, f)) = c.recv() {
            frames.push(f);
        }
        runs.push(
            frames
                .into_iter()
                .filter(|f| f["type"] != "heartbeat")
                .map(|f| normal(f, true))
                .collect(),
        );
    }
    (runs.remove(0), runs.remove(0))
}

#[test]
fn the_same_scenario_over_http_and_sse_and_through_the_session_loop_gives_the_same_frames() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let (d1, d2) = (Dir::new(), Dir::new());
    issue_credential(&d1, "alice");
    issue_credential(&d1, "bob");
    let direct = direct_run(&d1);
    let tokens = [issue_credential(&d2, "alice"), issue_credential(&d2, "bob")];
    let over_http = http_run(&d2, [tokens[0].as_str(), tokens[1].as_str()]);
    assert!(direct.0.len() > 10 && direct.1.len() > 10, "{direct:?}");
    // Responses come in request order in both. A stream's messages and caught_up arrive between them at moments that
    // depend on timing, so those are compared as a sorted set, not by position.
    let split = |frames: &Vec<Value>| -> (Vec<Value>, Vec<String>) {
        let responses = frames
            .iter()
            .filter(|f| f["type"] == "response")
            .cloned()
            .collect();
        let mut rest: Vec<String> = frames
            .iter()
            .filter(|f| f["type"] != "response")
            .map(|f| f.to_string())
            .collect();
        rest.sort();
        (responses, rest)
    };
    for (name, a, b) in [
        ("alice", &direct.0, &over_http.0),
        ("bob", &direct.1, &over_http.1),
    ] {
        let (ra, sa) = split(a);
        let (rb, sb) = split(b);
        assert!(ra.len() >= 10, "{name}: {ra:?}");
        assert_eq!(ra, rb, "{name}: responses");
        assert!(!sa.is_empty(), "{name}: a stream was seen");
        assert_eq!(sa, sb, "{name}: stream frames");
    }
    let seqs = |frames: &Vec<Value>| -> Vec<i64> {
        frames
            .iter()
            .filter(|f| {
                f["type"] == "response" && f["id"].as_str().is_some_and(|i| i.starts_with('p'))
            })
            .map(|f| f["result"]["seq"].as_i64().unwrap())
            .collect()
    };
    assert_eq!(seqs(&over_http.0), [1, 2, 3, 4]);
    assert_eq!(seqs(&over_http.1), [5, 6, 7, 8]);
}
