//! MCP over Streamable HTTP on the listener of `klatch-server http` (#1060, phase 4 of epic #1056): `POST /mcp` with one
//! JSON-RPC message per request (specification revision 2025-06-18), loopback only, the credential of phase 3 on every
//! request. The server runs as a child process against a temp store; a test is a client of plain sockets with a read
//! timeout on every one, and never reads an open event stream to its end (heartbeats keep resetting a timeout). Covered:
//! the credential matrix; `initialize` with the tools capability and no channel capability (channels are stdio only);
//! the nine tools acting as the credential's member (an argument that names someone else is ignored); notifications
//! answered `202`; JSON-RPC and HTTP errors; the Origin, Accept and protocol-version checks; sizes; the connection cap
//! shared with the wire routes; a credential revoked while a client goes on is refused at once.

// helpers in the shape of tests/http_wire.rs; not every one is used here
#![allow(dead_code)]

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
        let until = Instant::now() + Duration::from_secs(30);
        while Instant::now() < until {
            let text = std::fs::read_to_string(&server.log).unwrap_or_default();
            if let Some(rest) = text.split("http listening on ").nth(1)
                && let Some(addr) = rest.split_whitespace().next()
            {
                server.addr = addr.parse().expect("the logged address");
                return server;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("the server never listened");
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

const MCP: &str = "/mcp";

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

/// One JSON-RPC POST to `/mcp` as `member` with `token`, and the whole reply (a JSON answer is a short body that ends).
fn mcp_post(
    addr: SocketAddr,
    member: &str,
    token: &str,
    extra: &[(&str, &str)],
    body: &[u8],
) -> Reply {
    let b = bearer(token);
    let mut h = vec![
        ("X-Klatch-Member", member),
        ("Authorization", b.as_str()),
        ("Content-Type", "application/json"),
        ("Accept", "application/json, text/event-stream"),
    ];
    h.extend_from_slice(extra);
    exchange(addr, "POST", MCP, &h, body)
}

fn rpc(addr: SocketAddr, member: &str, token: &str, id: u64, method: &str, params: Value) -> Value {
    let body = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}).to_string();
    let r = mcp_post(addr, member, token, &[], body.as_bytes());
    assert_eq!(r.status, 200, "{method}: {}", r.body);
    assert!(
        r.header("content-type")
            .is_some_and(|c| c.starts_with("application/json")),
        "{:?}",
        r.headers
    );
    serde_json::from_str(&r.body).unwrap_or_else(|e| panic!("{:?}: {e}", r.body))
}

fn call(addr: SocketAddr, member: &str, token: &str, tool: &str, args: Value) -> Value {
    rpc(
        addr,
        member,
        token,
        7,
        "tools/call",
        json!({"name": tool, "arguments": args}),
    )["result"]
        .clone()
}

const PING: &[u8] = br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;

#[test]
fn every_mcp_request_needs_a_live_credential_for_the_member_named() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let token = issue_credential(&dir, "w-1");
    let _other = issue_credential(&dir, "w-2");
    let s = Server::started(&dir, &[]);
    let good = bearer(&token);
    let wrong = bearer(&"0".repeat(64));
    let cases: Vec<Vec<(&str, &str)>> = vec![
        vec![],
        vec![("X-Klatch-Member", "w-1")],
        vec![("Authorization", good.as_str())],
        vec![
            ("X-Klatch-Member", "w-1"),
            ("Authorization", wrong.as_str()),
        ],
        vec![("X-Klatch-Member", "w-2"), ("Authorization", good.as_str())],
        vec![("X-Klatch-Member", "w-1"), ("Authorization", "Basic abc")],
        // a typed name is no credential here either
        vec![("X-Klatch-Member", "alice")],
    ];
    for headers in &cases {
        let r = exchange(s.addr, "POST", MCP, headers, PING);
        assert_eq!(r.status, 401, "{headers:?}");
        assert_eq!(r.body, "unauthorized");
        assert!(!r.body.contains(&token) && !r.headers.iter().any(|(_, v)| v.contains(&token)));
    }
    let r = rpc(s.addr, "w-1", &token, 1, "ping", json!({}));
    assert_eq!(r["result"], json!({}));
    s.signal(libc::SIGTERM);
    let (_status, log) = s.finish();
    assert!(!log.contains(&token), "{log}");
}

#[test]
fn a_revoked_credential_is_refused_on_the_next_request() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let token = issue_credential(&dir, "w-1");
    let s = Server::started(&dir, &[]);
    assert_eq!(
        rpc(s.addr, "w-1", &token, 1, "ping", json!({}))["result"],
        json!({})
    );
    let store = klatch::member::custody::open_store(&klatch::member::db_path().unwrap()).unwrap();
    let gateway =
        klatch::gateway::Gateway::new(store, klatch::member::CliClock, klatch::gateway::AllowAll);
    gateway
        .admin_revoke_credential(&klatch::domain::Principal::unattributed("launcher"), "w-1")
        .unwrap();
    drop(gateway);
    let r = mcp_post(s.addr, "w-1", &token, &[], PING);
    assert_eq!(r.status, 401);
}

#[test]
fn initialize_offers_the_tools_and_never_the_channel_capability() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let token = issue_credential(&dir, "w-1");
    let s = Server::started(&dir, &[]);
    let r = rpc(
        s.addr,
        "w-1",
        &token,
        1,
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}),
    );
    let result = &r["result"];
    assert_eq!(r["id"], 1);
    assert_eq!(result["protocolVersion"], "2025-06-18");
    assert!(result["capabilities"]["tools"].is_object(), "{result}");
    // channels (notifications/claude/channel) are a stdio feature: nothing over HTTP can push one
    assert!(
        result["capabilities"].get("experimental").is_none(),
        "{result}"
    );
    assert!(!result.to_string().contains("claude/channel"), "{result}");
    assert_eq!(result["serverInfo"]["name"], "klatch");
    let instr = result["instructions"].as_str().unwrap();
    assert!(instr.contains("w-1"), "{instr}");
    let list = rpc(s.addr, "w-1", &token, 2, "tools/list", json!({}));
    let mut names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "klatch_asks",
            "klatch_claim",
            "klatch_invite",
            "klatch_join",
            "klatch_post",
            "klatch_read",
            "klatch_release",
            "klatch_wait",
            "klatch_who"
        ]
    );
}

#[test]
fn tools_act_as_the_member_of_the_credential_and_share_one_gateway() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let t1 = issue_credential(&dir, "w-1");
    let t2 = issue_credential(&dir, "w-2");
    let s = Server::started(&dir, &[]);
    for (m, t) in [("w-1", &t1), ("w-2", &t2)] {
        let r = call(s.addr, m, t, "klatch_join", json!({"room": "lobby"}));
        assert_eq!(r["isError"], false, "{r}");
    }
    // `member` and `sender` arguments are not who is calling
    let r = call(
        s.addr,
        "w-1",
        &t1,
        "klatch_post",
        json!({"room": "lobby", "body": "hello over mcp", "member": "w-2", "sender": "w-2"}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let read = call(s.addr, "w-2", &t2, "klatch_read", json!({"room": "lobby"}));
    assert_eq!(read["isError"], false, "{read}");
    let text = read["structuredContent"].to_string();
    assert!(
        text.contains("hello over mcp") && text.contains("w-1"),
        "{text}"
    );
    assert!(!text.contains("\"sender\":\"w-2\""), "{text}");
    // a wire session on the same listener sees the same store: the message has the next number for both transports
    let mut sse = Sse::open(s.addr, "w-2", &t2);
    assert_eq!(sse.frame()["type"], "hello");
    let seq = sse.post("lobby", "hello over sse");
    let again = call(
        s.addr,
        "w-1",
        &t1,
        "klatch_post",
        json!({"room":"lobby","body":"third"}),
    );
    assert_eq!(again["isError"], false, "{again}");
    assert_eq!(again["structuredContent"]["seq"].as_i64().unwrap(), seq + 1);
}

#[test]
fn a_notification_or_a_response_from_the_client_is_accepted_with_no_body() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let token = issue_credential(&dir, "w-1");
    let s = Server::started(&dir, &[]);
    for body in [
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":9,"result":{}}"#,
    ] {
        let r = mcp_post(s.addr, "w-1", &token, &[], body.as_bytes());
        assert_eq!(r.status, 202, "{body}");
        assert_eq!(r.body, "");
    }
}

#[test]
fn bad_json_a_batch_and_an_unknown_method_get_json_rpc_errors() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let token = issue_credential(&dir, "w-1");
    let s = Server::started(&dir, &[]);
    let r = mcp_post(s.addr, "w-1", &token, &[], b"{not json");
    assert_eq!(r.status, 400);
    let v: Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(v["error"]["code"], -32700, "{v}");
    assert_eq!(v["id"], Value::Null);
    // batches are gone from the 2025-06-18 revision
    let r = mcp_post(
        s.addr,
        "w-1",
        &token,
        &[],
        br#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#,
    );
    assert_eq!(r.status, 400);
    let v: Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(v["error"]["code"], -32600, "{v}");
    let r = mcp_post(s.addr, "w-1", &token, &[], b"");
    assert_eq!(r.status, 400);
    // an unknown method is a JSON-RPC error in a 200, as on stdio
    let v = rpc(s.addr, "w-1", &token, 3, "no/such", json!({}));
    assert_eq!(v["error"]["code"], -32601, "{v}");
    // pretty-printed JSON (newlines inside the body) is one message
    let pretty = "{\n \"jsonrpc\": \"2.0\",\n \"id\": 4,\n \"method\": \"ping\"\n}\n";
    let r = mcp_post(s.addr, "w-1", &token, &[], pretty.as_bytes());
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(serde_json::from_str::<Value>(&r.body).unwrap()["id"], 4);
}

#[test]
fn the_origin_accept_and_protocol_version_headers_are_checked() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let token = issue_credential(&dir, "w-1");
    let s = Server::started(&dir, &[]);
    // a page on another origin (DNS rebinding) is refused even with a valid credential
    for origin in [
        "http://evil.example",
        "http://127.0.0.1.evil.example:80",
        "null",
        "https://localhost.evil.example",
    ] {
        let r = mcp_post(s.addr, "w-1", &token, &[("Origin", origin)], PING);
        assert_eq!(r.status, 403, "{origin}");
    }
    for origin in [
        "http://127.0.0.1:3000",
        "http://localhost:8080",
        "http://[::1]:1",
        "https://localhost",
    ] {
        let r = mcp_post(s.addr, "w-1", &token, &[("Origin", origin)], PING);
        assert_eq!(r.status, 200, "{origin}: {}", r.body);
    }
    // the credential is checked first: a bad one is 401 whatever the origin
    let r = mcp_post(
        s.addr,
        "w-1",
        "nope",
        &[("Origin", "http://evil.example")],
        PING,
    );
    assert_eq!(r.status, 401);
    // the protocol version header, when sent, must be one this server speaks
    let r = mcp_post(
        s.addr,
        "w-1",
        &token,
        &[("MCP-Protocol-Version", "2025-06-18")],
        PING,
    );
    assert_eq!(r.status, 200);
    let r = mcp_post(
        s.addr,
        "w-1",
        &token,
        &[("MCP-Protocol-Version", "1999-01-01")],
        PING,
    );
    assert_eq!(r.status, 400);
    // a client that cannot take a JSON answer is not served one
    let b = bearer(&token);
    let r = exchange(
        s.addr,
        "POST",
        MCP,
        &[
            ("X-Klatch-Member", "w-1"),
            ("Authorization", b.as_str()),
            ("Accept", "text/html"),
        ],
        PING,
    );
    assert_eq!(r.status, 406);
}

#[test]
fn only_post_is_served_on_the_mcp_path_and_sizes_are_limited() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let token = issue_credential(&dir, "w-1");
    let s = Server::started(&dir, &[]);
    // no standalone event stream and no session to delete: both say so
    for method in ["GET", "DELETE", "PUT"] {
        let r = exchange(s.addr, method, MCP, &[], b"");
        assert_eq!(r.status, 405, "{method}");
        assert_eq!(r.header("allow"), Some("POST"));
    }
    let big = vec![b'x'; 1024 * 1024 + 1];
    let r = mcp_post(s.addr, "w-1", &token, &[], &big);
    assert_eq!(r.status, 413);
    let r = exchange(s.addr, "POST", "/mcp/extra", &[], b"{}");
    assert_eq!(r.status, 404);
    // the wire routes are still there beside it
    let mut sse = Sse::open(s.addr, "w-1", &token);
    assert_eq!(sse.frame()["type"], "hello");
}

#[test]
fn mcp_requests_count_against_the_connection_cap_of_the_listener() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let t = issue_credential(&dir, "w-1");
    let s = Server::started(&dir, &["--max-connections", "2"]);
    let mut a = Sse::open(s.addr, "w-1", &t);
    let mut b = Sse::open(s.addr, "w-1", &t);
    a.frame();
    b.frame();
    let r = mcp_post(s.addr, "w-1", &t, &[], PING);
    assert_eq!(r.status, 503);
    drop(a);
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        let r = mcp_post(s.addr, "w-1", &t, &[], PING);
        if r.status != 503 || Instant::now() > until {
            assert_eq!(r.status, 200, "the slot of the closed connection is freed");
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(b);
}

#[test]
fn a_wait_asked_for_too_long_is_a_json_rpc_error_and_a_short_one_times_out() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let t = issue_credential(&dir, "w-1");
    let s = Server::started(&dir, &[]);
    let v = rpc(
        s.addr,
        "w-1",
        &t,
        1,
        "tools/call",
        json!({"name":"klatch_wait","arguments":{"rooms":["lobby"],"timeout":9999}}),
    );
    assert_eq!(v["error"]["code"], -32602, "{v}");
    let r = call(
        s.addr,
        "w-1",
        &t,
        "klatch_wait",
        json!({"rooms":["lobby"],"timeout":1}),
    );
    assert_eq!(r["isError"], false, "{r}");
}
