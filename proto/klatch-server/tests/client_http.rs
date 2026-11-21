//! The Rust client over `klatch-server http` (#1122, feature `client-http`). The server is a child process against a
//! throwaway store under /tmp; each member has a credential issued in that store (the same call the helper
//! `examples/issue_test_credential.rs` makes, here in process).
//!
//! Verified: the conformance scenario of `klatch/tests/client_scenario` runs unchanged over HTTP and server-sent events
//! (`home()`, `connect()` and `ATTRIBUTION` below are the only difference: a member here is `verified`); a refused
//! credential; urls that are not plain loopback http are refused before anything connects; a header value that could
//! split a request is refused; a reconnect resumes from the last sequence number with no gap and no repeat (the broken
//! stream is injected, not provoked from the platform); a server that went away is an error, not a hang; and a stand-in
//! server shows the headers, the `data:` lines as frames, and the retry on a full inbox (429).
//!
//! Every wait has a deadline, and no stream is ever read to its end: it has none.

#![cfg(unix)]

use klatch::client::{Client, Error, HttpFault, HttpOptions, PostRequest};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[path = "../../klatch/tests/client_scenario/mod.rs"]
mod client_scenario;

/// What the hello says of a member here: a credential makes it verified.
pub const ATTRIBUTION: &str = "verified";

static N: AtomicU32 = AtomicU32::new(0);
/// Issuing a credential reads the environment of this process: one test at a time may set it.
static ENV: Mutex<()> = Mutex::new(());

/// A throwaway store with its own `klatch-server http`, started on first use.
pub struct Home {
    dir: PathBuf,
    server: Mutex<Option<(Child, SocketAddr)>>,
    tokens: Mutex<HashMap<String, String>>,
}

impl Home {
    fn new() -> Home {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = PathBuf::from(format!("/tmp/kx{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["", "h"] {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(dir.join(sub))
                .unwrap();
        }
        Home {
            dir,
            server: Mutex::new(None),
            tokens: Mutex::new(HashMap::new()),
        }
    }

    fn log(&self) -> PathBuf {
        self.dir.join("err.log")
    }

    /// Start the server once and wait until its log names the address.
    fn addr(&self) -> SocketAddr {
        let mut server = self.server.lock().unwrap();
        if server.is_none() {
            let home = self.dir.join("h");
            let child = Command::new(env!("CARGO_BIN_EXE_klatch-server"))
                .args(["http", "--listen", "127.0.0.1:0"])
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("HOME", &home)
                .env("KLATCH_HOME", &home)
                .env("KLATCH_KEY_DIR", home.join("keys"))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::from(std::fs::File::create(self.log()).unwrap()))
                .spawn()
                .expect("start klatch-server");
            let until = Instant::now() + Duration::from_secs(30);
            let addr = loop {
                let text = std::fs::read_to_string(self.log()).unwrap_or_default();
                if let Some(rest) = text.split("http listening on ").nth(1)
                    && let Some(a) = rest.split_whitespace().next()
                {
                    break a.parse().expect("the logged address");
                }
                assert!(Instant::now() < until, "the server never listened");
                std::thread::sleep(Duration::from_millis(25));
            };
            *server = Some((child, addr));
        }
        server.as_ref().unwrap().1
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr())
    }

    /// The credential of `member`, issued in this store on first use.
    fn token(&self, member: &str) -> String {
        let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let mut tokens = self.tokens.lock().unwrap();
        if let Some(t) = tokens.get(member) {
            return t.clone();
        }
        let store_home = self.dir.join("h");
        unsafe {
            std::env::set_var("KLATCH_HOME", &store_home);
            std::env::set_var("KLATCH_KEY_DIR", store_home.join("keys"));
            std::env::remove_var("KLATCH_NAME");
            std::env::remove_var("KLATCH_IDENTITY_FILE");
        }
        let store =
            klatch::member::custody::open_store(&klatch::member::db_path().unwrap()).unwrap();
        let gateway = klatch::gateway::Gateway::new(
            store,
            klatch::member::CliClock,
            klatch::gateway::AllowAll,
        );
        let spec = klatch::gateway::CredentialSpec {
            member,
            clearance: "public",
            ttl: 3600,
            max_life: None,
            parent: None,
        };
        let token = gateway
            .admin_issue_credential(&klatch::domain::Principal::unattributed("launcher"), &spec)
            .unwrap()
            .to_text()
            .to_string();
        tokens.insert(member.into(), token.clone());
        token
    }

    /// Stop the server now (its streams close).
    fn stop(&self) {
        if let Some((mut child, _)) = self.server.lock().unwrap().take() {
            unsafe {
                libc::kill(child.id() as libc::pid_t, libc::SIGKILL);
            }
            let _ = child.wait();
        }
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        self.stop();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub fn home() -> Home {
    Home::new()
}

/// A member of the store, over HTTP, with its credential.
pub fn connect(h: &Home, name: &str) -> Client {
    let token = h.token(name);
    Client::connect_http(&h.url(), name, &token).unwrap()
}

fn quick() -> HttpOptions {
    HttpOptions::new()
        .connect_timeout(Duration::from_secs(5))
        .reconnect_attempts(3)
        .reconnect_delay(Duration::from_millis(10))
}

fn api(e: Error) -> klatch::client::ApiError {
    match e {
        Error::Api(a) => a,
        other => panic!("not an api error: {other}"),
    }
}

fn session_error(e: Error) -> String {
    match e {
        Error::Session(m) => m,
        other => panic!("not a session error: {other}"),
    }
}

// ---- what only HTTP has ---------------------------------------------------------------------------------------

#[test]
fn a_credential_gives_a_verified_member_and_a_wrong_one_is_refused_without_echoing_it() {
    let h = home();
    let token = h.token("w-1");
    let _other = h.token("w-2");
    let url = h.url();
    let mut w = Client::connect_http(&url, "w-1", &token).unwrap();
    assert_eq!(
        (w.member.as_str(), w.attribution.as_str()),
        ("w-1", "verified")
    );
    w.post(PostRequest::new("r", "from a worker")).unwrap();
    for (member, bad) in [
        ("w-1", "0".repeat(64)),
        ("w-2", token.clone()),
        ("alice", "x".into()),
    ] {
        let e = Client::connect_http(&url, member, &bad)
            .map(|_| ())
            .unwrap_err();
        let a = api(e);
        assert_eq!(a.code, "identity.refused");
        assert!(!a.message.contains(&token) && !a.message.contains(&bad));
    }
}

#[test]
fn a_url_that_is_not_plain_loopback_http_is_refused_before_anything_connects() {
    // a listener that would see any connection: none may arrive
    let spy = TcpListener::bind("127.0.0.1:0").unwrap();
    spy.set_nonblocking(true).unwrap();
    let port = spy.local_addr().unwrap().port();
    for url in [
        format!("https://127.0.0.1:{port}"),
        format!("ftp://127.0.0.1:{port}"),
        format!("127.0.0.1:{port}"),
        format!("http://127.0.0.2:{port}"),
        format!("http://localhost.evil.test:{port}"),
        format!("http://127.0.0.1.evil.test:{port}"),
        format!("http://user@127.0.0.1:{port}"),
        format!("http://127.0.0.1:{port}@evil.test"),
        format!("http://0.0.0.0:{port}"),
        format!("http://[::]:{port}"),
        format!("http://[::ffff:127.0.0.1]:{port}"),
        format!("http://192.168.1.5:{port}"),
        format!("http://127.0.0.1:{port}/v1/stream"),
        format!("http://127.0.0.1:{port}?x=1"),
        "http://127.0.0.1".to_string(),
        "http://127.0.0.1:0".to_string(),
        "http://127.0.0.1:99999".to_string(),
        "http://127.0.0.1:".to_string(),
        String::new(),
    ] {
        let e = Client::connect_http(&url, "w-1", &"0".repeat(64))
            .map(|_| ())
            .unwrap_err();
        let m = session_error(e);
        assert!(m.contains("loopback") || m.contains("url"), "{url}: {m}");
    }
    assert!(
        matches!(spy.accept(), Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock),
        "something connected"
    );
}

#[test]
fn a_member_or_token_that_could_split_a_request_is_refused_before_connecting() {
    let spy = TcpListener::bind("127.0.0.1:0").unwrap();
    spy.set_nonblocking(true).unwrap();
    let url = format!("http://127.0.0.1:{}", spy.local_addr().unwrap().port());
    for (member, token) in [
        ("w-1\r\nX-Evil: 1", "t"),
        ("w-1", "t\r\nX-Evil: 1"),
        ("w-1", "t\nx"),
        ("w 1", "t"),
        ("", "t"),
        ("w-1", ""),
    ] {
        let e = Client::connect_http(&url, member, token)
            .map(|_| ())
            .unwrap_err();
        session_error(e);
    }
    assert!(spy.accept().is_err(), "something connected");
}

#[test]
fn a_server_nobody_listens_on_is_a_session_error_within_the_deadline() {
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let start = Instant::now();
    let e = Client::connect_http_with(
        &format!("http://127.0.0.1:{port}"),
        "w-1",
        "t",
        &quick().connect_timeout(Duration::from_secs(2)),
    )
    .map(|_| ())
    .unwrap_err();
    session_error(e);
    assert!(start.elapsed() < Duration::from_secs(10));
}

#[test]
fn close_ends_the_session_and_answers_zero_and_the_server_stays_up() {
    let h = home();
    let a = connect(&h, "alice");
    assert_eq!(a.close().unwrap(), 0);
    let mut b = connect(&h, "bob");
    b.post(PostRequest::new("r", "after")).unwrap();
}

#[test]
fn a_broken_stream_is_reopened_and_resumes_from_the_last_seq_with_no_gap_and_no_repeat() {
    let h = home();
    let fault = HttpFault::new();
    let token = h.token("alice");
    let mut a =
        Client::connect_http_with(&h.url(), "alice", &token, &quick().fault(&fault)).unwrap();
    let mut b = connect(&h, "bob");
    a.join("r").unwrap();
    b.join("r").unwrap();
    b.post(PostRequest::new("r", "one")).unwrap();
    a.subscribe("r", 0).unwrap();
    let first = a.next_message(Duration::from_secs(10)).unwrap().unwrap();
    assert_eq!((first.seq, first.body.as_str()), (1, "one"));
    // the stream breaks (injected); two posts land while it is down or being reopened
    fault.fail_next_read();
    b.post(PostRequest::new("r", "two")).unwrap();
    b.post(PostRequest::new("r", "three")).unwrap();
    let got: Vec<(u64, String)> = (0..2)
        .map(|_| {
            let m = a
                .next_message(Duration::from_secs(20))
                .unwrap()
                .expect("a message after the reconnect");
            (m.seq, m.body)
        })
        .collect();
    assert_eq!(got, [(2, "two".to_string()), (3, "three".to_string())]);
    // nothing is replayed, and the session is still good for requests and for later messages
    assert!(
        a.next_message(Duration::from_millis(500))
            .unwrap()
            .is_none()
    );
    b.post(PostRequest::new("r", "four")).unwrap();
    let m = a.next_message(Duration::from_secs(10)).unwrap().unwrap();
    assert_eq!((m.seq, m.body.as_str()), (4, "four"));
    a.post(PostRequest::new("r", "from a")).unwrap();
}

#[test]
fn a_server_that_went_away_is_a_session_error_not_a_hang() {
    let h = home();
    let fault = HttpFault::new();
    let token = h.token("alice");
    let mut a =
        Client::connect_http_with(&h.url(), "alice", &token, &quick().fault(&fault)).unwrap();
    a.post(PostRequest::new("r", "x")).unwrap();
    h.stop();
    fault.fail_next_read();
    let start = Instant::now();
    let e = a.post(PostRequest::new("r", "y")).unwrap_err();
    session_error(e);
    assert!(start.elapsed() < Duration::from_secs(25));
}

// ---- a stand-in server: the bytes the client sends, and a 429 ---------------------------------------------------

/// What the stand-in saw: the head of each request (lower-cased header names) and the body of each POST.
#[derive(Default)]
struct Seen {
    gets: Vec<HashMap<String, String>>,
    posts: Vec<(HashMap<String, String>, String)>,
}

/// A server that answers `GET /v1/stream` with an event stream (session `S1`, a hello, a comment, an event with `id:`
/// and `event:` lines and a frame split over two `data:` lines is not produced by klatch, so one `data:` line each) and
/// answers each `POST /v1/frames` with the next of `statuses`; on a 202 it writes a response to the request on the
/// stream, as the real server does.
struct Stub {
    addr: SocketAddr,
    seen: Arc<Mutex<Seen>>,
    done: Arc<AtomicBool>,
}

impl Stub {
    fn start(statuses: Vec<u16>) -> Stub {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Seen::default()));
        let done = Arc::new(AtomicBool::new(false));
        let stream: Arc<Mutex<Option<TcpStream>>> = Arc::new(Mutex::new(None));
        let (s2, d2) = (seen.clone(), done.clone());
        std::thread::spawn(move || {
            let mut statuses = statuses.into_iter();
            let until = Instant::now() + Duration::from_secs(60);
            while !d2.load(Ordering::SeqCst) && Instant::now() < until {
                let Ok((conn, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                conn.set_nonblocking(false).unwrap();
                conn.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut r = BufReader::new(conn.try_clone().unwrap());
                let mut line = String::new();
                if r.read_line(&mut line).unwrap_or(0) == 0 {
                    continue;
                }
                let method = line.split(' ').next().unwrap_or("").to_string();
                let mut headers = HashMap::new();
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    let h = h.trim_end();
                    if h.is_empty() {
                        break;
                    }
                    let (n, v) = h.split_once(':').unwrap();
                    headers.insert(n.to_ascii_lowercase(), v.trim().to_string());
                }
                let mut conn = conn;
                if method == "GET" {
                    s2.lock().unwrap().gets.push(headers);
                    conn.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: close\r\nX-Klatch-Session: S1\r\n\r\n").unwrap();
                    conn.write_all(b"event: hello\ndata: {\"v\":1,\"type\":\"hello\",\"member\":\"w-1\",\"attribution\":\"verified\",\"clearance\":\"public\"}\n\n: a comment\n\n").unwrap();
                    *stream.lock().unwrap() = Some(conn);
                } else {
                    let n: usize = headers
                        .get("content-length")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    let mut body = vec![0u8; n];
                    r.read_exact(&mut body).unwrap();
                    let body = String::from_utf8(body).unwrap();
                    s2.lock().unwrap().posts.push((headers, body.clone()));
                    let status = statuses.next().unwrap_or(202);
                    let phrase = if status == 202 { "Accepted" } else { "No" };
                    let _ = conn.write_all(
                        format!("HTTP/1.1 {status} {phrase}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes(),
                    );
                    if status == 202
                        && let Some(s) = stream.lock().unwrap().as_mut()
                    {
                        let id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
                            .as_str()
                            .unwrap()
                            .to_string();
                        let _ = s.write_all(
                            format!("event: response\nid: x\ndata: {{\"v\":1,\"type\":\"response\",\"id\":\"{id}\",\"ok\":true,\"result\":{{\"seq\":7,\"id\":\"m7\"}}}}\n\n").as_bytes(),
                        );
                    }
                }
            }
        });
        Stub { addr, seen, done }
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
    }
}

#[test]
fn the_credential_the_member_and_the_session_are_sent_and_data_lines_are_the_frames() {
    let stub = Stub::start(vec![]);
    let mut c = Client::connect_http(&stub.url(), "w-1", "tok-123").unwrap();
    assert_eq!(
        (c.member.as_str(), c.attribution.as_str()),
        ("w-1", "verified")
    );
    let (seq, id) = c.post(PostRequest::new("r", "hi")).unwrap();
    assert_eq!((seq, id.as_str()), (7, "m7"));
    let seen = stub.seen.lock().unwrap();
    assert_eq!(seen.gets.len(), 1);
    let g = &seen.gets[0];
    assert_eq!(g["authorization"], "Bearer tok-123");
    assert_eq!(g["x-klatch-member"], "w-1");
    assert_eq!(g["accept"], "text/event-stream");
    assert_eq!(seen.posts.len(), 1);
    let (h, body) = &seen.posts[0];
    assert_eq!(h["authorization"], "Bearer tok-123");
    assert_eq!(h["x-klatch-member"], "w-1");
    assert_eq!(h["x-klatch-session"], "S1");
    assert_eq!(h["content-type"], "application/x-ndjson");
    assert!(
        body.ends_with('\n') && body.matches('\n').count() == 1,
        "{body:?}"
    );
}

#[test]
fn a_full_inbox_is_retried_until_the_server_accepts_the_frame() {
    let stub = Stub::start(vec![429, 429, 429]);
    let mut c = Client::connect_http(&stub.url(), "w-1", "tok").unwrap();
    let (seq, _) = c.post(PostRequest::new("r", "hi")).unwrap();
    assert_eq!(seq, 7);
    assert_eq!(stub.seen.lock().unwrap().posts.len(), 4);
}

#[test]
fn a_refusal_that_is_not_a_full_inbox_is_an_error_and_is_not_retried() {
    let stub = Stub::start(vec![500]);
    let mut c = Client::connect_http(&stub.url(), "w-1", "tok").unwrap();
    let e = c.post(PostRequest::new("r", "hi")).unwrap_err();
    assert!(session_error(e).contains("500"));
    assert_eq!(stub.seen.lock().unwrap().posts.len(), 1);
}
