//! `klatch-server channel --webhook` end to end (#1058, epic #1056): a child process on a throwaway store and a port the
//! system chooses (port 0, read from the child's standard error), driven with scripted JSON-RPC on stdio and raw HTTP
//! POSTs from this test over loopback, the way `gh webhook forward --url` would send them. Nothing here talks to GitHub,
//! runs `gh`, or touches the live store or keychain (a cleared environment, a temporary home and key directory).
//!
//! The fixtures in tests/fixtures are hand-written to GitHub's documented shapes, NOT recordings.
//!
//! Verified: a correctly signed event gives exactly one channel notification with the expected meta and no payload text;
//! a wrongly signed or unsigned one gives 401 and nothing; hostile text in a title, body and branch appears nowhere in
//! standard output or standard error; a non-loopback address and a missing secret stop the server at startup; the wrong
//! path and method and an oversized body are refused. Not verified: that Claude Code shows the events, or a real
//! `gh webhook forward` run.

use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

static N: AtomicU32 = AtomicU32::new(0);
const SECRET: &str = "e2e-test-secret";
const SECRET_ENV: &str = "KLATCH_TEST_HOOK_SECRET";

struct Home(PathBuf);

impl Home {
    fn new() -> Self {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("klatch-server-hook-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Home(dir)
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn hmac(key: &[u8], msg: &[u8]) -> String {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).unwrap();
    mac.update(msg);
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{name}.json")))
        .unwrap()
}

struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<Value>,
    /// Every line the child wrote to standard output and standard error, for the leak checks.
    stdout_text: Arc<Mutex<String>>,
    stderr_text: Arc<Mutex<String>>,
    notifications: Vec<Value>,
    addr: SocketAddr,
    next_id: i64,
}

fn command(home: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_klatch-server"));
    c.args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home)
        .env("KLATCH_HOME", home)
        .env("KLATCH_KEY_DIR", home.join("keys"))
        .env("KLATCH_NAME", "alice")
        .env("KLATCH_SCOPE", "bot:channel")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

/// Read the child's standard output on a thread: every line is kept in `text`, and each is one JSON value, sent on.
fn read_json_lines(stdout: ChildStdout, text: Arc<Mutex<String>>) -> Receiver<Value> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            text.lock().unwrap().push_str(&format!("{line}\n"));
            let value: Value = serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("standard output carries JSON only: {line:?}: {e}"));
            if tx.send(value).is_err() {
                break;
            }
        }
    });
    rx
}

/// The address of an intake line of standard error (`webhook intake on ADDR ...`), or `None` for any other line.
fn intake_address(line: &str) -> Option<String> {
    let rest = line.split("webhook intake on ").nth(1)?;
    Some(rest.split(' ').next().unwrap().to_string())
}

/// Read the child's standard error on a thread: every line is kept in `text`, and the port comes from the line that
/// reports the intake.
fn read_address(stderr: ChildStderr, text: Arc<Mutex<String>>) -> Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            text.lock().unwrap().push_str(&format!("{line}\n"));
            if let Some(addr) = intake_address(&line) {
                let _ = tx.send(addr);
            }
        }
    });
    rx
}

impl Server {
    fn start(home: &Path) -> Server {
        let mut child = command(
            home,
            &[
                "channel",
                "--webhook",
                "127.0.0.1:0",
                "--webhook-secret-env",
                SECRET_ENV,
            ],
        )
        .env(SECRET_ENV, SECRET)
        .spawn()
        .expect("start klatch-server");
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let stdout_text = Arc::new(Mutex::new(String::new()));
        let stderr_text = Arc::new(Mutex::new(String::new()));
        let rx = read_json_lines(stdout, stdout_text.clone());
        let prx = read_address(stderr, stderr_text.clone());
        let addr: SocketAddr = prx
            .recv_timeout(Duration::from_secs(20))
            .expect("the server reports its address")
            .parse()
            .unwrap();
        assert!(addr.ip().is_loopback());
        let stdin = child.stdin.take();
        let mut server = Server {
            child,
            stdin,
            rx,
            stdout_text,
            stderr_text,
            notifications: Vec::new(),
            addr,
            next_id: 1,
        };
        let init = server.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}),
        );
        assert!(init["result"].is_object(), "{init}");
        server.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        server
    }

    fn send(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(v) if v["id"] == json!(id) => return v,
                Ok(v) => self.notifications.push(v),
                Err(e) => panic!("no answer to {method}: {e:?}"),
            }
        }
    }

    /// The channel notifications seen so far plus those that arrive within `window`.
    fn events(&mut self, window: Duration) -> Vec<Value> {
        let end = Instant::now() + window;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(v) => self.notifications.push(v),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
            }
        }
        self.notifications
            .iter()
            .filter(|v| v["method"] == "notifications/claude/channel")
            .cloned()
            .collect()
    }

    fn wait_for_events(&mut self, n: usize, window: Duration) -> Vec<Value> {
        let end = Instant::now() + window;
        while Instant::now() < end {
            let have = self.events(Duration::from_millis(100));
            if have.len() >= n {
                return have;
            }
        }
        self.events(Duration::from_millis(1))
    }

    /// POST raw bytes to a path with the given extra headers; returns the status and the whole answer.
    fn post(&self, path: &str, headers: &[(&str, String)], body: &[u8]) -> (u16, String) {
        let mut raw = format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n",
            body.len()
        );
        for (k, v) in headers {
            raw.push_str(&format!("{k}: {v}\r\n"));
        }
        raw.push_str("\r\n");
        let mut bytes = raw.into_bytes();
        bytes.extend_from_slice(body);
        self.raw(&bytes)
    }

    fn raw(&self, bytes: &[u8]) -> (u16, String) {
        let mut s = TcpStream::connect(self.addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let _ = s.write_all(bytes);
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        let text = String::from_utf8_lossy(&out).into_owned();
        let status = text
            .split(' ')
            .nth(1)
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        (status, text)
    }

    /// A correctly signed delivery of a fixture.
    fn deliver(&self, event: &str, fixture_name: &str) -> (u16, String) {
        let body = fixture(fixture_name);
        self.post(
            "/github",
            &[
                ("X-GitHub-Event", event.to_string()),
                (
                    "X-Hub-Signature-256",
                    format!("sha256={}", hmac(SECRET.as_bytes(), &body)),
                ),
            ],
            &body,
        )
    }

    fn stop(mut self) -> (String, String) {
        self.stdin.take();
        let _ = self.child.wait();
        std::thread::sleep(Duration::from_millis(100));
        let out = self.stdout_text.lock().unwrap().clone();
        let err = self.stderr_text.lock().unwrap().clone();
        (out, err)
    }
}

const QUIET: Duration = Duration::from_millis(1500);
const WAIT: Duration = Duration::from_secs(10);

fn keys(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

#[test]
fn a_signed_pull_request_event_gives_exactly_one_notification_with_meta_and_no_payload_text() {
    let home = Home::new();
    let mut s = Server::start(&home.0);
    let (status, _) = s.deliver("pull_request", "pull_request_opened");
    assert_eq!(status, 200);
    let events = s.wait_for_events(1, WAIT);
    assert_eq!(events.len(), 1, "{events:?}");
    let p = &events[0]["params"];
    assert_eq!(keys(&p["meta"]), ["event", "pr", "sha_short", "state"]);
    assert_eq!(p["meta"]["event"], "pull_request");
    assert_eq!(p["meta"]["pr"], "12");
    assert_eq!(p["meta"]["sha_short"], "a1b2c3d");
    assert_eq!(p["meta"]["state"], "open");
    let content = p["content"].as_str().unwrap();
    assert!(
        content.starts_with("[klatch] GitHub: pull request #12 opened"),
        "{content}"
    );
    for text in ["Add a thing", "Please review", "feature-x", "example/repo"] {
        assert!(!content.contains(text), "{content}");
    }
    // still exactly one a moment later
    assert_eq!(s.events(QUIET).len(), 1);
    let (out, err) = s.stop();
    assert!(!out.contains("Add a thing") && !err.contains("Add a thing"));
    assert!(!out.contains(SECRET) && !err.contains(SECRET));
}

#[test]
fn a_superseded_cancelled_suite_and_a_main_suite_follow_the_rules() {
    let home = Home::new();
    let mut s = Server::start(&home.0);
    assert_eq!(
        s.deliver("check_suite", "check_suite_cancelled_superseded")
            .0,
        200
    );
    assert_eq!(s.deliver("check_suite", "check_suite_main").0, 200);
    let events = s.wait_for_events(2, WAIT);
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[0]["params"]["meta"]["conclusion"], "superseded");
    assert!(
        events[0]["params"]["content"]
            .as_str()
            .unwrap()
            .contains("not a failure")
    );
    assert_eq!(events[1]["params"]["meta"]["state"], "main");
    assert!(events[1]["params"]["meta"].get("pr").is_none());
    s.stop();
}

#[test]
fn a_wrongly_signed_or_unsigned_event_is_401_and_nothing_is_emitted() {
    let home = Home::new();
    let mut s = Server::start(&home.0);
    let body = fixture("pull_request_opened");
    let wrong = format!("sha256={}", hmac(b"not-the-secret", &body));
    let (status, _) = s.post(
        "/github",
        &[
            ("X-GitHub-Event", "pull_request".into()),
            ("X-Hub-Signature-256", wrong),
        ],
        &body,
    );
    assert_eq!(status, 401);
    let (status, _) = s.post(
        "/github",
        &[("X-GitHub-Event", "pull_request".into())],
        &body,
    );
    assert_eq!(status, 401);
    assert!(s.events(QUIET).is_empty());
    // the server is still fine afterwards
    assert_eq!(s.deliver("pull_request", "pull_request_opened").0, 200);
    assert_eq!(s.wait_for_events(1, WAIT).len(), 1);
    s.stop();
}

#[test]
fn hostile_text_in_a_title_body_or_branch_appears_nowhere_in_the_output() {
    let home = Home::new();
    let mut s = Server::start(&home.0);
    assert_eq!(s.deliver("pull_request", "hostile_pull_request").0, 200);
    let events = s.wait_for_events(1, WAIT);
    assert_eq!(events.len(), 1);
    let (out, err) = s.stop();
    for needle in [
        "IGNORE", "PREVIOUS", "rm -rf", "script", "SYSTEM", "keychain", "evil", "mallory",
    ] {
        assert!(!out.contains(needle), "{needle} in standard output: {out}");
        assert!(!err.contains(needle), "{needle} in standard error: {err}");
    }
}

#[test]
fn the_wrong_path_the_wrong_method_and_an_oversized_body_are_refused() {
    let home = Home::new();
    let mut s = Server::start(&home.0);
    let body = fixture("pull_request_opened");
    let sig = format!("sha256={}", hmac(SECRET.as_bytes(), &body));
    let good_headers = [
        ("X-GitHub-Event", "pull_request".to_string()),
        ("X-Hub-Signature-256", sig),
    ];
    assert_eq!(s.post("/elsewhere", &good_headers, &body).0, 404);
    assert_eq!(s.raw(b"GET /github HTTP/1.1\r\nHost: x\r\n\r\n").0, 405);
    let (status, text) = s.raw(b"PUT /github HTTP/1.1\r\nContent-Length: 0\r\n\r\n");
    assert_eq!(status, 405);
    assert!(text.contains("Allow: POST"));
    // a declared length over 1 MiB is refused before the body is read
    let (status, _) = s.raw(
        format!(
            "POST /github HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            2 * 1024 * 1024
        )
        .as_bytes(),
    );
    assert_eq!(status, 413);
    assert!(s.events(QUIET).is_empty());
    s.stop();
}

#[test]
fn a_signed_ping_is_accepted_and_emits_nothing() {
    let home = Home::new();
    let mut s = Server::start(&home.0);
    assert_eq!(s.deliver("ping", "ping").0, 200);
    assert!(s.events(QUIET).is_empty());
    s.stop();
}

fn exits_with_error(home: &Path, args: &[&str], envs: &[(&str, &str)]) -> String {
    let mut c = command(home, args);
    for (k, v) in envs {
        c.env(k, v);
    }
    let mut child = c.spawn().unwrap();
    drop(child.stdin.take());
    let mut err = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut err)
        .unwrap();
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    let status = child.wait().unwrap();
    assert!(!status.success(), "expected a startup error: {err}");
    assert_eq!(out, "", "nothing on standard output at a refusal");
    err
}

#[test]
fn a_non_loopback_address_is_refused_at_startup_naming_the_prerequisites() {
    let home = Home::new();
    for addr in ["0.0.0.0:0", "192.168.1.9:9000", "[::]:0", "localhost:9000"] {
        let err = exits_with_error(
            &home.0,
            &[
                "channel",
                "--webhook",
                addr,
                "--webhook-secret-env",
                SECRET_ENV,
            ],
            &[(SECRET_ENV, SECRET)],
        );
        assert!(
            err.contains("refusing") && err.contains("#157") && err.contains("#161"),
            "{addr}: {err}"
        );
        assert!(!err.contains(SECRET));
    }
}

#[test]
fn a_missing_secret_is_a_startup_error() {
    let home = Home::new();
    let err = exits_with_error(&home.0, &["channel", "--webhook", "127.0.0.1:0"], &[]);
    assert!(err.contains("secret"), "{err}");
    // the named variable is not set
    let err = exits_with_error(
        &home.0,
        &[
            "channel",
            "--webhook",
            "127.0.0.1:0",
            "--webhook-secret-env",
            SECRET_ENV,
        ],
        &[],
    );
    assert!(err.contains(SECRET_ENV), "{err}");
}

#[test]
fn the_secret_is_never_a_command_line_value() {
    let home = Home::new();
    for flag in ["--webhook-secret", "--secret"] {
        let err = exits_with_error(
            &home.0,
            &[
                "channel",
                "--webhook",
                "127.0.0.1:0",
                flag,
                "hunter2hunter2",
            ],
            &[],
        );
        assert!(!err.contains("hunter2"), "{err}");
    }
    // a secret typed after --webhook-secret-env is rejected as not a name and not echoed
    let err = exits_with_error(
        &home.0,
        &[
            "channel",
            "--webhook",
            "127.0.0.1:0",
            "--webhook-secret-env",
            "hunter2-hunter2!",
        ],
        &[],
    );
    assert!(!err.contains("hunter2"), "{err}");
}

#[test]
fn without_the_webhook_option_no_socket_is_opened() {
    // the phase 1 behaviour is unchanged: no listener line, the server still works over stdio
    let home = Home::new();
    let mut child = command(&home.0, &["channel"]).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}})
    )
    .unwrap();
    drop(stdin);
    let mut err = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut err)
        .unwrap();
    let _ = child.wait();
    assert!(!err.contains("webhook"), "{err}");
}
