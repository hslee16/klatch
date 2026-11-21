//! `klatch-server daemon --socket PATH` (#102), run as a child process against a throwaway store and a short socket
//! directory under /tmp (a socket path is limited to about 100 bytes), driven with scripted session frames. Every test
//! has its own home, key directory and socket; a child gets a cleared environment, so the live store and the keychain
//! are never touched.
//!
//! What this verifies: the identity tiers (no identify: `unmapped-process`, unattributed; a typed name: unattributed
//! under that name; a credential: verified; a bad credential: refused and closed), two concurrent connections through
//! one gateway with dense sequence numbers, the socket file (mode 0600, directory checks, a path that is too long, a
//! stale file, a live daemon), the limits (frame size, concurrent connections, a connection that sends nothing),
//! SIGTERM and SIGINT, garbage on one connection, a store that cannot be opened, and a conformance run: the same
//! scripted scenario through the session loop in this process (what `klatch serve --stdio` runs) and through the
//! daemon produces the same frames, with the same sequence numbers.
//!
//! What it cannot verify here: a connection from another uid (the check is a unit test on the evidence function in
//! `src/daemon.rs`, with a fake credential), and a socket directory owned by another uid.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

static N: AtomicU32 = AtomicU32::new(0);
/// The library reads its environment (the conformance run and the credential setup open a store in this process): one
/// test at a time may set it.
static ENV: Mutex<()> = Mutex::new(());

/// A short directory under /tmp with a store home and a socket directory in it.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Dir {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = PathBuf::from(format!("/tmp/kd{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["", "h", "s"] {
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
    fn sock(&self) -> PathBuf {
        self.0.join("s").join("d.sock")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Daemon {
    child: Child,
    sock: PathBuf,
}

impl Daemon {
    fn start(dir: &Dir, args: &[&str]) -> Daemon {
        Self::start_at(dir, &dir.sock(), args)
    }

    fn start_at(dir: &Dir, sock: &Path, args: &[&str]) -> Daemon {
        let child = Command::new(env!("CARGO_BIN_EXE_klatch-server"))
            .arg("daemon")
            .arg("--socket")
            .arg(sock)
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", dir.home())
            .env("KLATCH_HOME", dir.home())
            .env("KLATCH_KEY_DIR", dir.home().join("keys"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start klatch-server");
        Daemon {
            child,
            sock: sock.to_path_buf(),
        }
    }

    /// Wait until the socket accepts a connection.
    fn ready(&self) {
        let until = Instant::now() + Duration::from_secs(20);
        while Instant::now() < until {
            if UnixStream::connect(&self.sock).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("the daemon never listened");
    }

    fn started(dir: &Dir, args: &[&str]) -> Daemon {
        let d = Self::start(dir, args);
        d.ready();
        d
    }

    /// The exit status once the child has ended (panics after the timeout), and what it wrote to standard error.
    fn finish(mut self) -> (std::process::ExitStatus, String) {
        let until = Instant::now() + Duration::from_secs(20);
        let status = loop {
            if let Some(s) = self.child.try_wait().unwrap() {
                break s;
            }
            assert!(Instant::now() < until, "the daemon did not stop");
            std::thread::sleep(Duration::from_millis(20));
        };
        let mut err = String::new();
        if let Some(mut e) = self.child.stderr.take() {
            std::io::Read::read_to_string(&mut e, &mut err).unwrap();
        }
        (status, err)
    }

    fn signal(&self, sig: libc::c_int) {
        unsafe {
            libc::kill(self.child.id() as libc::pid_t, sig);
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        unsafe {
            libc::kill(self.child.id() as libc::pid_t, libc::SIGKILL);
        }
        let _ = self.child.wait();
    }
}

/// One scripted connection.
struct Conn {
    w: UnixStream,
    r: BufReader<UnixStream>,
    /// Frames that arrived while a response was awaited.
    extra: Vec<Value>,
    next: u32,
}

impl Conn {
    fn open(sock: &Path) -> Conn {
        let w = UnixStream::connect(sock).expect("connect");
        w.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        let r = BufReader::new(w.try_clone().unwrap());
        Conn {
            w,
            r,
            extra: Vec::new(),
            next: 0,
        }
    }

    /// A connection and its hello.
    fn hello(sock: &Path) -> (Conn, Value) {
        let mut c = Conn::open(sock);
        let hello = c.recv().expect("hello");
        assert_eq!(hello["type"], "hello", "{hello}");
        (c, hello)
    }

    /// A connection that has said who it is by a typed name.
    fn named(sock: &Path, name: &str) -> Conn {
        let (mut c, _) = Conn::hello(sock);
        let r = c.request("identify", json!({ "name": name }));
        assert_eq!(r["ok"], true, "{r}");
        c
    }

    /// A line, whole. A daemon that has closed the connection already (a frame over the cap) makes the write fail.
    fn send_line(&mut self, line: &str) {
        let _ = self.w.write_all(line.as_bytes());
        let _ = self.w.write_all(b"\n");
    }

    fn send(&mut self, v: &Value) {
        self.send_line(&v.to_string());
    }

    fn recv(&mut self) -> Option<Value> {
        let mut line = String::new();
        match self.r.read_line(&mut line) {
            Ok(0) => None,
            Ok(_) => Some(serde_json::from_str(&line).unwrap_or_else(|e| panic!("{line:?}: {e}"))),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::ConnectionReset) => None,
            Err(e) => panic!("read: {e}"),
        }
    }

    /// Send a request and return its response; other frames are kept in `extra`.
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = format!("r{}", self.next);
        self.send(&json!({"v":1,"type":"request","id":id,"method":method,"params":params}));
        loop {
            let f = self.recv().unwrap_or_else(|| panic!("closed before the response to {method}"));
            if f["type"] == "response" && f["id"] == id.as_str() {
                return f;
            }
            self.extra.push(f);
        }
    }

    /// Frames until the connection closes.
    fn rest(&mut self) -> Vec<Value> {
        let mut all = std::mem::take(&mut self.extra);
        while let Some(f) = self.recv() {
            all.push(f);
        }
        all
    }

    fn post(&mut self, room: &str, body: &str) -> i64 {
        let r = self.request("post", json!({"room": room, "body": body}));
        assert_eq!(r["ok"], true, "{r}");
        r["result"]["seq"].as_i64().unwrap()
    }
}

#[test]
fn two_concurrent_connections_share_one_gateway_with_dense_sequence_numbers() {
    let dir = Dir::new();
    let d = Daemon::started(&dir, &[]);
    let mut seqs: Vec<i64> = Vec::new();
    let threads: Vec<_> = ["alice", "bob"]
        .into_iter()
        .map(|name| {
            let sock = d.sock.clone();
            std::thread::spawn(move || {
                let mut c = Conn::named(&sock, name);
                assert_eq!(c.request("join", json!({"room": "r"}))["ok"], true);
                (0..10)
                    .map(|i| c.post("r", &format!("{name}-{i}")))
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    for t in threads {
        seqs.extend(t.join().unwrap());
    }
    seqs.sort();
    assert_eq!(seqs, (1..=20).collect::<Vec<i64>>());
    // a third connection reads all of it, in order, as a stream
    let mut c = Conn::named(&d.sock, "carol");
    c.request("join", json!({"room": "r"}));
    let r = c.request("subscribe", json!({"room": "r", "from_seq": 0}));
    assert_eq!(r["ok"], true, "{r}");
    let mut got = Vec::new();
    while got.len() < 20 {
        let f = c.recv().expect("stream");
        if f["type"] == "message" {
            assert_eq!(f["attribution"], "unattributed");
            got.push(f["seq"].as_i64().unwrap());
        }
    }
    assert_eq!(got, (1..=20).collect::<Vec<i64>>());
}

#[test]
fn a_connection_that_does_not_identify_is_unmapped_process_and_unattributed() {
    let dir = Dir::new();
    let d = Daemon::started(&dir, &[]);
    let (mut c, hello) = Conn::hello(&d.sock);
    assert_eq!(hello["member"], "unmapped-process");
    assert_eq!(hello["attribution"], "unattributed");
    assert_eq!(c.request("join", json!({"room": "r"}))["ok"], true);
    assert_eq!(c.post("r", "hi"), 1);
    let r = c.request("history", json!({"room": "r", "last": 5}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(r.to_string().contains("unmapped-process"), "{r}");
}

#[test]
fn a_typed_name_is_unattributed_and_a_bad_one_is_refused_without_echo() {
    let dir = Dir::new();
    let d = Daemon::started(&dir, &[]);
    let (mut c, _) = Conn::hello(&d.sock);
    let r = c.request("identify", json!({"name": "alice"}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["result"]["member"], "alice");
    assert_eq!(r["result"]["attribution"], "unattributed");
    c.request("join", json!({"room": "r"}));
    c.post("r", "hello");
    let r = c.request("history", json!({"room": "r", "last": 5}));
    assert!(r.to_string().contains("alice"), "{r}");
    // a reserved or malformed name: refused, the connection closes, nothing falls back to another name
    for bad in ["admin", "canary", "a b", "x/y"] {
        let (mut c, _) = Conn::hello(&d.sock);
        let r = c.request("identify", json!({ "name": bad }));
        assert_eq!(r["ok"], false, "{r}");
        assert!(!r.to_string().contains(&format!("\"{bad}\"")), "{r}");
        assert!(c.rest().is_empty(), "the connection closes after a refusal");
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
    // the human's gateway (not one members act through, which refuses an admin act)
    let store = klatch::member::custody::open_store(&klatch::member::db_path().unwrap()).unwrap();
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
        .unwrap();
    token.to_text().to_string()
}

#[test]
fn a_credential_gives_a_verified_member_and_a_bad_one_is_refused_and_closed() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Dir::new();
    let token = issue_credential(&dir, "w-1");
    let d = Daemon::started(&dir, &[]);
    let (mut c, _) = Conn::hello(&d.sock);
    let r = c.request("identify", json!({"member": "w-1", "token": token}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["result"]["member"], "w-1");
    assert_eq!(r["result"]["attribution"], "verified");
    c.request("join", json!({"room": "r"}));
    c.post("r", "from a worker");
    let r = c.request("history", json!({"room": "r", "last": 5}));
    assert!(r.to_string().contains("\"verified\""), "{r}");

    // a wrong token, another member's name with this token, text that is no token, and a typed name together with a
    // credential: each is refused and the connection closes; none falls back to a typed name
    let other = "0".repeat(64);
    for params in [
        json!({"member": "w-1", "token": other}),
        json!({"member": "w-2", "token": token}),
        json!({"member": "w-1", "token": "nope"}),
        json!({"member": "w-1", "token": token, "name": "alice"}),
        json!({"token": token}),
    ] {
        let (mut c, _) = Conn::hello(&d.sock);
        let r = c.request("identify", params.clone());
        assert_eq!(r["ok"], false, "{params}: {r}");
        assert!(!r.to_string().contains(&token), "{r}");
        assert!(c.rest().is_empty(), "closed after a refusal");
    }
}

#[test]
fn identify_after_another_request_is_an_ordinary_unknown_method() {
    let dir = Dir::new();
    let d = Daemon::started(&dir, &[]);
    let (mut c, _) = Conn::hello(&d.sock);
    assert_eq!(c.request("join", json!({"room": "r"}))["ok"], true);
    let r = c.request("identify", json!({"name": "alice"}));
    assert_eq!(r["ok"], false, "{r}");
    // still the member the connection began as
    assert_eq!(c.post("r", "x"), 1);
    let r = c.request("history", json!({"room": "r", "last": 5}));
    assert!(r.to_string().contains("unmapped-process"), "{r}");
}

#[test]
fn the_socket_is_mode_0600_and_it_is_removed_on_sigterm() {
    let dir = Dir::new();
    let d = Daemon::started(&dir, &[]);
    let mode = std::fs::metadata(&d.sock).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let sock = d.sock.clone();
    // a session open at the time is closed, and the process exits by itself, cleanly
    let (mut open, _) = Conn::hello(&sock);
    d.signal(libc::SIGTERM);
    let (status, err) = d.finish();
    assert!(status.success(), "{status:?}: {err}");
    assert!(!sock.exists(), "the socket is removed on a clean exit");
    assert!(open.rest().is_empty());
}

#[test]
fn sigint_stops_the_daemon_the_same_way() {
    let dir = Dir::new();
    let d = Daemon::started(&dir, &[]);
    let sock = d.sock.clone();
    d.signal(libc::SIGINT);
    let (status, err) = d.finish();
    assert!(status.success(), "{status:?}: {err}");
    assert!(!sock.exists());
}

#[test]
fn a_socket_directory_others_can_write_to_is_refused_by_name() {
    for mode in [0o777, 0o770, 0o757] {
        let dir = Dir::new();
        let sdir = dir.0.join("s");
        std::fs::set_permissions(&sdir, std::fs::Permissions::from_mode(mode)).unwrap();
        let (status, err) = Daemon::start(&dir, &[]).finish();
        assert_eq!(status.code(), Some(1), "{mode:o}: {err}");
        assert!(err.contains("not safe"), "{mode:o}: {err}");
        assert!(err.contains("writable"), "{mode:o}: {err}");
        assert!(!dir.sock().exists());
    }
}

#[test]
fn a_missing_socket_directory_and_a_file_in_the_way_are_refused() {
    let dir = Dir::new();
    let missing = dir.0.join("nope").join("d.sock");
    let (status, err) = Daemon::start_at(&dir, &missing, &[]).finish();
    assert_eq!(status.code(), Some(1), "{err}");
    assert!(err.contains("directory"), "{err}");
    // a regular file at the socket path is never removed
    std::fs::write(dir.sock(), b"precious").unwrap();
    let (status, err) = Daemon::start(&dir, &[]).finish();
    assert_eq!(status.code(), Some(1), "{err}");
    assert!(err.contains("not a socket"), "{err}");
    assert_eq!(std::fs::read(dir.sock()).unwrap(), b"precious");
}

#[test]
fn a_path_over_the_socket_limit_is_refused_naming_the_limit() {
    let dir = Dir::new();
    let long = dir.0.join("s").join("a".repeat(150));
    let (status, err) = Daemon::start_at(&dir, &long, &[]).finish();
    assert_eq!(status.code(), Some(1), "{err}");
    assert!(err.contains("too long"), "{err}");
    assert!(err.contains("bytes"), "{err}");
    assert!(err.contains("limit"), "{err}");
}

#[test]
fn a_stale_socket_file_is_replaced_and_a_live_daemon_is_never_stolen_from() {
    let dir = Dir::new();
    // a dead daemon's file: bound, then abandoned
    drop(std::os::unix::net::UnixListener::bind(dir.sock()).unwrap());
    assert!(dir.sock().exists());
    let a = Daemon::started(&dir, &[]);
    let (mut c, hello) = Conn::hello(&a.sock);
    assert_eq!(hello["type"], "hello");
    assert_eq!(c.request("join", json!({"room": "r"}))["ok"], true);

    // a second daemon on the same path: refused, and the first one still serves
    let (status, err) = Daemon::start(&dir, &[]).finish();
    assert_eq!(status.code(), Some(1), "{err}");
    assert!(err.contains("already listening"), "{err}");
    assert_eq!(c.post("r", "still here"), 1);
    let (mut c2, _) = Conn::hello(&a.sock);
    assert_eq!(c2.request("join", json!({"room": "r"}))["ok"], true);
}

#[test]
fn a_frame_over_the_cap_is_refused_and_other_connections_are_not_affected() {
    let dir = Dir::new();
    let d = Daemon::started(&dir, &[]);
    let mut good = Conn::named(&d.sock, "bob");
    let mut bad = Conn::named(&d.sock, "alice");
    let huge = "x".repeat(2 * 1024 * 1024);
    bad.send_line(&format!(
        "{{\"v\":1,\"type\":\"request\",\"id\":\"big\",\"method\":\"post\",\"params\":{{\"room\":\"r\",\"body\":\"{huge}\"}}}}"
    ));
    let f = bad.recv().expect("a frame saying why");
    assert!(
        f["type"] == "refusal" || (f["type"] == "response" && f["ok"] == false),
        "{f}"
    );
    assert!(!f.to_string().contains("xxxxxxxx"), "the refusal never echoes the frame");
    assert_eq!(good.request("join", json!({"room": "r"}))["ok"], true);
    assert_eq!(good.post("r", "fine"), 1);
}

#[test]
fn a_first_line_over_the_cap_closes_the_connection_with_a_refusal() {
    let dir = Dir::new();
    let d = Daemon::started(&dir, &[]);
    let (mut c, _) = Conn::hello(&d.sock);
    c.send_line(&"y".repeat(2 * 1024 * 1024));
    let f = c.recv().expect("a refusal");
    assert_eq!(f["type"], "refusal", "{f}");
    assert!(c.rest().is_empty());
    let (mut other, _) = Conn::hello(&d.sock);
    assert_eq!(other.request("join", json!({"room": "r"}))["ok"], true);
}

#[test]
fn the_connection_cap_refuses_the_excess_with_a_frame_and_frees_a_slot_when_one_closes() {
    let dir = Dir::new();
    let d = Daemon::started(&dir, &["--max-connections", "2"]);
    let (a, _) = Conn::hello(&d.sock);
    let (b, _) = Conn::hello(&d.sock);
    let mut third = Conn::open(&d.sock);
    let f = third.recv().expect("a refusal frame");
    assert_eq!(f["type"], "refusal", "{f}");
    assert_eq!(f["rule"], "daemon.connections");
    assert!(third.rest().is_empty(), "then the connection closes");
    drop(a);
    // the slot frees when the daemon sees the close; try until it does
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let mut c = Conn::open(&d.sock);
        let f = c.recv().expect("a frame");
        if f["type"] == "hello" {
            break;
        }
        assert!(Instant::now() < until, "the slot never freed");
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(b);
}

#[test]
fn a_connection_that_sends_nothing_is_closed_after_the_first_frame_timeout() {
    let dir = Dir::new();
    let d = Daemon::started(&dir, &["--first-frame-secs", "1"]);
    let started = Instant::now();
    let (mut silent, _) = Conn::hello(&d.sock);
    assert!(silent.rest().is_empty(), "closed with nothing sent");
    assert!(started.elapsed() < Duration::from_secs(15));
    // a connection that did speak is not dropped for silence afterwards
    let mut talker = Conn::named(&d.sock, "alice");
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(talker.request("join", json!({"room": "r"}))["ok"], true);
}

#[test]
fn garbage_on_one_connection_gets_an_error_frame_and_does_not_affect_others() {
    let dir = Dir::new();
    let d = Daemon::started(&dir, &[]);
    let mut good = Conn::named(&d.sock, "bob");
    let (mut bad, _) = Conn::hello(&d.sock);
    bad.send_line("this is not json {{{");
    let f = bad.recv().expect("an error frame");
    assert!(
        f["type"] == "refusal" || (f["type"] == "response" && f["ok"] == false),
        "{f}"
    );
    assert!(!f.to_string().contains("this is not json"), "{f}");
    assert_eq!(good.request("join", json!({"room": "r"}))["ok"], true);
    assert_eq!(good.post("r", "ok"), 1);
}

#[test]
fn a_store_that_cannot_be_opened_stops_the_daemon_before_it_listens() {
    let dir = Dir::new();
    // not a database
    std::fs::write(dir.home().join("rooms.db"), vec![7u8; 4096]).unwrap();
    let (status, err) = Daemon::start(&dir, &[]).finish();
    assert_eq!(status.code(), Some(1), "{err}");
    assert!(!dir.sock().exists());
    assert!(!err.is_empty());
}

/// The scenario both transports run: two members, one session each, in turn.
fn scenario(member: &str) -> Vec<Value> {
    let req = |id: &str, method: &str, params: Value| {
        json!({"v":1,"type":"request","id":id,"method":method,"params":params})
    };
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
    s.push(req("who", "claim", json!({"room": "conf", "path": format!("{member}.rs")})));
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

/// The scenario through the session loop in this process, as `klatch serve --stdio` runs it (a gateway of the member
/// setup, no daemon): the frames after the hello, for the two members in turn.
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
        let input: String = scenario(name)
            .iter()
            .map(|f| format!("{f}\n"))
            .collect();
        let config = klatch::session::SessionConfig::default();
        let mut out = Vec::new();
        klatch::session::Session::new(&m.gateway, m.principal(), &config)
            .serve(std::io::Cursor::new(input.into_bytes()), &mut out)
            .unwrap();
        let frames: Vec<Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(frames[0]["type"], "hello");
        assert_eq!(frames[0]["member"], name);
        runs.push(frames[1..].iter().cloned().map(|f| normal(f, true)).collect());
    }
    (runs.remove(0), runs.remove(0))
}

fn daemon_run(dir: &Dir) -> (Vec<Value>, Vec<Value>) {
    let d = Daemon::started(dir, &[]);
    let mut runs = Vec::new();
    for name in ["alice", "bob"] {
        let (mut c, _) = Conn::hello(&d.sock);
        let id = c.request("identify", json!({ "name": name }));
        assert_eq!(id["result"]["member"], name);
        assert_eq!(id["result"]["attribution"], "unattributed");
        for f in scenario(name) {
            c.send(&f);
        }
        c.w.shutdown(std::net::Shutdown::Write).unwrap();
        let frames = c.rest();
        runs.push(frames.into_iter().map(|f| normal(f, true)).collect());
    }
    (runs.remove(0), runs.remove(0))
}

#[test]
fn the_same_scenario_through_the_daemon_and_through_the_session_loop_gives_the_same_frames() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let direct = direct_run(&Dir::new());
    let over_daemon = daemon_run(&Dir::new());
    assert!(direct.0.len() > 10 && direct.1.len() > 10, "{direct:?}");
    for (name, a, b) in [
        ("alice", &direct.0, &over_daemon.0),
        ("bob", &direct.1, &over_daemon.1),
    ] {
        assert_eq!(a.len(), b.len(), "{name}: {a:#?}\nvs\n{b:#?}");
        for (x, y) in a.iter().zip(b) {
            assert_eq!(x, y, "{name}");
        }
    }
    // bob's posts follow alice's: the sequence numbers go on from 5, in both
    let seqs = |frames: &Vec<Value>| -> Vec<i64> {
        frames
            .iter()
            .filter(|f| f["type"] == "response" && f["id"].as_str().is_some_and(|i| i.starts_with('p')))
            .map(|f| f["result"]["seq"].as_i64().unwrap())
            .collect()
    };
    assert_eq!(seqs(&over_daemon.0), [1, 2, 3, 4]);
    assert_eq!(seqs(&over_daemon.1), [5, 6, 7, 8]);
}
