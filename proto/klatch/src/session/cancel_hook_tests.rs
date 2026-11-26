//! The cancel hook of `Backend` (#1562): a look already inside the backend can be cancelled. The shared gateway's owner
//! thread is held by a job the test controls, so a look queued behind it blocks exactly as long as the test says; a
//! cancel must end the wait at once with the cancelled outcome, and only for the look it names. Every wait here has a
//! deadline, so a regression fails an assertion instead of hanging.

use super::acp::AcpServer;
use super::{Backend, Outcome, Session, SessionConfig};
use crate::domain::Principal;
use crate::gateway::{AllowAll, Gateway, SharedBackend, SharedGateway, SystemClock};
use crate::session::methods::Call;
use crate::store::SqliteStore;
use crate::store::test_dir::{TempDb, temp_db};
use serde_json::{Value, json};
use std::io::{self, BufRead, Read, Write};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

/// How long a test waits for anything before it calls the run broken.
const WAIT: Duration = Duration::from_secs(5);
/// How soon after a cancel the wait must have ended. The owner is still held when this is measured.
const PROMPT: Duration = Duration::from_secs(2);

fn shared(tag: &str) -> (SharedGateway, TempDb) {
    let db = temp_db(tag);
    let path = db.to_path_buf();
    let gw = SharedGateway::spawn(move || {
        let store = SqliteStore::open(&path, &crate::store::insecure_test_key())?;
        Ok(Gateway::new(store, SystemClock, AllowAll))
    })
    .unwrap();
    (gw, db)
}

/// Hold the owner thread with a job that ends when `release` is sent (or after `WAIT`, so a failed test frees it).
struct HeldOwner {
    release: Sender<()>,
    job: Option<std::thread::JoinHandle<()>>,
}

impl HeldOwner {
    fn hold(gw: &SharedGateway) -> Self {
        let (started_tx, started) = channel();
        let (release, release_rx) = channel::<()>();
        let gw = gw.clone();
        let job = std::thread::spawn(move || {
            let _ = gw.run_for_tests(move |_| {
                let _ = started_tx.send(());
                let _ = release_rx.recv_timeout(WAIT);
            });
        });
        started.recv_timeout(WAIT).expect("the owner never started");
        Self {
            release,
            job: Some(job),
        }
    }
    fn release(mut self) {
        let _ = self.release.send(());
        if let Some(job) = self.job.take() {
            job.join().unwrap();
        }
    }
}

fn look(id: &str) -> Call {
    Call {
        id: id.into(),
        method: "who".into(),
        value: json!({"room": "r"}),
    }
}

/// A look on its own thread; the outcome arrives on the channel.
fn start_look(backend: &Arc<SharedBackend>, id: &str) -> Receiver<Outcome> {
    let (tx, rx) = channel();
    let (backend, id) = (backend.clone(), id.to_string());
    std::thread::spawn(move || {
        let who = Principal::unattributed("lib");
        let _ = tx.send(backend.handle_cancellable(&who, look(&id), None));
    });
    rx
}

/// Wait until the look `id` is in flight (registered), so the cancel is for a look that is inside the backend.
fn wait_in_flight(backend: &Arc<SharedBackend>, want: usize) {
    let end = Instant::now() + WAIT;
    while backend.in_flight() < want {
        assert!(Instant::now() < end, "the look never went in flight");
        std::thread::yield_now();
    }
}

fn is_cancelled(o: &Outcome) -> bool {
    matches!(o, Outcome::Cancelled)
}

#[test]
fn a_cancel_during_a_blocked_look_returns_the_cancelled_outcome_within_a_short_deadline() {
    let (gw, _db) = shared("cancel-blocked");
    let backend = Arc::new(gw.session_backend());
    let owner = HeldOwner::hold(&gw);
    let rx = start_look(&backend, "a");
    wait_in_flight(&backend, 1);
    let begun = Instant::now();
    assert!(
        backend.canceller().cancel("a"),
        "a look in flight was not cancelled"
    );
    let got = rx.recv_timeout(PROMPT).expect("the cancelled look hung");
    assert!(is_cancelled(&got), "not a cancelled outcome");
    assert!(begun.elapsed() < PROMPT);
    // The owner was held the whole time: the wait ended because of the cancel, not because the owner finished.
    owner.release();
    // The owner is healthy afterwards: a fresh look completes.
    let who = Principal::unattributed("lib");
    assert!(matches!(
        backend.handle_cancellable(&who, look("b"), None),
        Outcome::Done(_)
    ));
}

#[test]
fn a_cancelled_request_that_had_not_started_is_never_run_by_the_owner() {
    // The owner meets a cancelled job and does nothing: a request that was never carried out leaves no effect. A post
    // stands in for any request with an effect: cancelled while queued it must not land; the same post uncancelled does.
    let (gw, _db) = shared("cancel-skip");
    let backend = Arc::new(gw.session_backend());
    let who = Principal::unattributed("lib");
    let say = |id: &str, method: &str, value: Value| {
        backend.handle(
            &who,
            Call {
                id: id.into(),
                method: method.into(),
                value,
            },
        )
    };
    say("j", "join", json!({"room": "r"}));
    let post = |id: &str, body: &str| Call {
        id: id.into(),
        method: "post".into(),
        value: json!({"room": "r", "body": body}),
    };
    let owner = HeldOwner::hold(&gw);
    let (tx, rx) = channel();
    let b2 = backend.clone();
    let who2 = who.clone();
    let queued = post("p1", "marker-cancelled");
    std::thread::spawn(move || {
        let _ = tx.send(b2.handle_cancellable(&who2, queued, None));
    });
    wait_in_flight(&backend, 1);
    assert!(backend.canceller().cancel("p1"));
    assert!(is_cancelled(&rx.recv_timeout(PROMPT).unwrap()));
    owner.release();
    let kept = backend.handle_cancellable(&who, post("p2", "marker-kept"), None);
    assert!(matches!(kept, Outcome::Done(_)));
    let history = format!(
        "{:?}",
        say("h", "history", json!({"room": "r", "last": 10}))
    );
    assert!(
        history.contains("marker-kept"),
        "control post missing: {history}"
    );
    assert!(
        !history.contains("marker-cancelled"),
        "the owner ran a request that was cancelled before it started: {history}"
    );
}

#[test]
fn a_cancel_after_the_look_finished_is_a_no_op() {
    let (gw, _db) = shared("cancel-after");
    let backend = Arc::new(gw.session_backend());
    let who = Principal::unattributed("lib");
    let out = backend.handle_cancellable(&who, look("a"), None);
    assert!(matches!(out, Outcome::Done(ref f) if !f.is_empty()));
    assert!(
        !backend.canceller().cancel("a"),
        "a cancel after completion claimed to cancel something"
    );
    assert_eq!(backend.in_flight(), 0, "a finished look stayed registered");
    // And it does not poison the next look with the same id.
    assert!(matches!(
        backend.handle_cancellable(&who, look("a"), None),
        Outcome::Done(_)
    ));
}

#[test]
fn a_cancel_for_another_request_id_does_nothing() {
    let (gw, _db) = shared("cancel-wrong-id");
    let backend = Arc::new(gw.session_backend());
    let owner = HeldOwner::hold(&gw);
    let rx = start_look(&backend, "a");
    wait_in_flight(&backend, 1);
    assert!(!backend.canceller().cancel("not-a"));
    assert!(!backend.canceller().cancel(""));
    assert!(
        rx.recv_timeout(Duration::from_millis(300)).is_err(),
        "a cancel for another id ended the look"
    );
    owner.release();
    let got = rx.recv_timeout(WAIT).expect("the look never finished");
    assert!(
        matches!(got, Outcome::Done(_)),
        "the look was not completed"
    );
}

#[test]
fn two_concurrent_looks_cancel_one_the_other_completes_normally() {
    let (gw, _db) = shared("cancel-two");
    let backend = Arc::new(gw.session_backend());
    let owner = HeldOwner::hold(&gw);
    let a = start_look(&backend, "a");
    let b = start_look(&backend, "b");
    wait_in_flight(&backend, 2);
    assert!(backend.canceller().cancel("a"));
    assert!(is_cancelled(&a.recv_timeout(PROMPT).expect("a hung")));
    assert!(
        b.recv_timeout(Duration::from_millis(300)).is_err(),
        "cancelling a ended b"
    );
    owner.release();
    let got = b.recv_timeout(WAIT).expect("b never finished");
    assert!(
        matches!(got, Outcome::Done(ref f) if !f.is_empty()),
        "b was cancelled or empty"
    );
}

#[test]
fn another_connections_look_with_the_same_id_is_not_cancelled() {
    // Ids are the connection's own: two sessions may both call a request "1". A cancel through one backend handle
    // never reaches a look of another handle.
    let (gw, _db) = shared("cancel-two-conns");
    let one = Arc::new(gw.session_backend());
    // A clone is how a daemon hands each connection its handle.
    let two = Arc::new((*one).clone());
    let owner = HeldOwner::hold(&gw);
    let a = start_look(&one, "1");
    let b = start_look(&two, "1");
    wait_in_flight(&one, 1);
    wait_in_flight(&two, 1);
    assert!(one.canceller().cancel("1"));
    assert!(is_cancelled(&a.recv_timeout(PROMPT).unwrap()));
    assert!(b.recv_timeout(Duration::from_millis(300)).is_err());
    // The other connection's own look is registered under its own table and is cancelled by its own hook.
    assert!(two.canceller().cancel("1"));
    assert!(is_cancelled(&b.recv_timeout(PROMPT).unwrap()));
    owner.release();
}

#[test]
fn the_default_hook_reports_not_cancelled_and_a_look_runs_to_the_end() {
    let g = Gateway::new(
        SqliteStore::open_in_memory().unwrap(),
        SystemClock,
        AllowAll,
    );
    assert!(!Backend::canceller(&g).cancel("a"));
    assert!(!Backend::canceller(&g).cancel(""));
    let who = Principal::unattributed("lib");
    assert!(matches!(
        Backend::handle_cancellable(&g, &who, look("a"), None),
        Outcome::Done(ref f) if !f.is_empty()
    ));
}

#[test]
fn a_cancel_storm_is_bounded_and_cancels_the_look_once() {
    let (gw, _db) = shared("cancel-storm");
    let backend = Arc::new(gw.session_backend());
    let owner = HeldOwner::hold(&gw);
    let rx = start_look(&backend, "a");
    wait_in_flight(&backend, 1);
    let hook = backend.canceller();
    let begun = Instant::now();
    let told = (0..100).filter(|_| hook.cancel("a")).count();
    assert_eq!(told, 1, "the look was cancelled {told} times");
    assert!(begun.elapsed() < PROMPT, "100 cancels were not bounded");
    assert!(is_cancelled(&rx.recv_timeout(PROMPT).unwrap()));
    owner.release();
}

/// Threads of this process, or `None` where it cannot be counted.
fn thread_count() -> Option<usize> {
    if let Ok(dir) = std::fs::read_dir("/proc/self/task") {
        return Some(dir.count());
    }
    let out = std::process::Command::new("ps")
        .args(["-M", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    let lines = String::from_utf8_lossy(&out.stdout).lines().count();
    lines.checked_sub(1)
}

#[test]
fn two_hundred_cancel_cycles_leave_no_thread_behind() {
    let (gw, _db) = shared("cancel-leak");
    let backend = Arc::new(gw.session_backend());
    // One cycle first, so any one-time thread (a lazily started helper) is counted in the baseline.
    let cycle = |backend: &Arc<SharedBackend>, n: usize| {
        let owner = HeldOwner::hold(&gw);
        let id = format!("c{n}");
        let rx = start_look(backend, &id);
        wait_in_flight(backend, 1);
        assert!(backend.canceller().cancel(&id));
        assert!(is_cancelled(&rx.recv_timeout(PROMPT).unwrap()));
        owner.release();
        assert_eq!(backend.in_flight(), 0);
    };
    cycle(&backend, 0);
    let Some(before) = thread_count() else {
        panic!("threads cannot be counted on this platform");
    };
    for n in 1..=200 {
        cycle(&backend, n);
    }
    let after = thread_count().unwrap();
    // Other tests run in this process and start and end their own threads; a leak of one per cycle would add 200.
    assert!(
        after <= before + 40,
        "threads grew from {before} to {after} over 200 cancel cycles"
    );
}

/// The agent's input: one chunk per line, from the test. Waits are bounded.
struct ChanInput {
    rx: Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
}

impl Read for ChanInput {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = self.fill_buf()?.len().min(out.len());
        out[..n].copy_from_slice(&self.fill_buf()?[..n]);
        self.consume(n);
        Ok(n)
    }
}

impl BufRead for ChanInput {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.pos >= self.buf.len() {
            match self.rx.recv_timeout(WAIT) {
                Ok(bytes) => self.buf = bytes,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => self.buf.clear(),
                Err(_) => return Err(io::Error::new(io::ErrorKind::TimedOut, "no input")),
            }
            self.pos = 0;
        }
        Ok(&self.buf[self.pos..])
    }
    fn consume(&mut self, n: usize) {
        self.pos += n;
    }
}

struct LineSink(Sender<String>, Vec<u8>);

impl Write for LineSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        for b in bytes {
            if *b == b'\n' {
                let _ = self.0.send(String::from_utf8_lossy(&self.1).into_owned());
                self.1.clear();
            } else {
                self.1.push(*b);
            }
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn an_acp_cancel_ends_a_prompt_blocked_in_the_backend_while_the_owner_is_still_held() {
    let (gw, _db) = shared("cancel-acp");
    let backend = Arc::new(gw.session_backend());
    let owner = HeldOwner::hold(&gw);
    let (input_tx, input_rx) = channel::<Vec<u8>>();
    let (lines_tx, lines) = channel::<String>();
    let served = {
        let backend = backend.clone();
        std::thread::spawn(move || {
            let who = Principal::unattributed("lib");
            let config = SessionConfig::default();
            let session = Session::over(&*backend, &who, &config);
            let input = ChanInput {
                rx: input_rx,
                buf: Vec::new(),
                pos: 0,
            };
            AcpServer::new(&session).serve(input, LineSink(lines_tx, Vec::new()))
        })
    };
    let send = |v: Value| input_tx.send(format!("{v}\n").into_bytes()).unwrap();
    send(json!({"jsonrpc":"2.0","id":1,"method":"session/new","params":{}}));
    send(
        json!({"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{
        "sessionId":"klatch-librarian-1","prompt":[{"type":"text","text":"who r"}]}}),
    );
    wait_in_flight(&backend, 1);
    let begun = Instant::now();
    send(
        json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":"klatch-librarian-1"}}),
    );
    let mut seen: Vec<Value> = Vec::new();
    while !seen.iter().any(|m| m["id"] == 3) {
        let left = PROMPT.saturating_sub(begun.elapsed());
        let line = lines
            .recv_timeout(left)
            .unwrap_or_else(|_| panic!("the cancelled prompt hung; saw {seen:?}"));
        seen.push(serde_json::from_str(&line).unwrap());
    }
    let reply = seen.iter().find(|m| m["id"] == 3).unwrap();
    assert_eq!(reply["result"]["stopReason"], "cancelled", "{seen:?}");
    assert!(
        !seen.iter().any(|m| m["method"] == "session/update"),
        "a partial answer was sent: {seen:?}"
    );
    owner.release();
    drop(input_tx);
    served.join().unwrap().unwrap();
    assert_eq!(backend.in_flight(), 0);
}
