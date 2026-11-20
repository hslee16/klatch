//! The guarded session (#147, #146): a consumer that stops reading is detected and refused instead of stalling the
//! session, and the input reader does not outlive the session when the transport gives a way to stop it.

use super::live::{ChanReader, SharedOut, fast};
use super::*;
use crate::session::Closer;
use crate::store::SqliteStore as Sqlite;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

type Gate = Arc<(Mutex<bool>, Condvar)>;

fn open_gate(gate: &Gate) {
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
}

/// An output whose writes block until the gate opens, as a pipe does when its reader has stopped.
struct StuckOut {
    gate: Gate,
    sink: Arc<Mutex<Vec<u8>>>,
}

impl Write for StuckOut {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let mut open = self.gate.0.lock().unwrap();
        while !*open {
            open = self.gate.1.wait(open).unwrap();
        }
        self.sink.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// An output that has gone away.
struct BrokenOut;

impl Write for BrokenOut {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// An input that never ends by itself: a read blocks until the sender is dropped, then reports the end. It tells when
/// it is dropped, which is when the reader thread that owns it has ended.
struct NeverEnds {
    wake: Receiver<()>,
    dropped: Arc<AtomicBool>,
}

impl Read for NeverEnds {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        let _ = self.wake.recv();
        Ok(0)
    }
}

impl Drop for NeverEnds {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

fn new_db(tag: &str) -> crate::store::test_dir::TempDb {
    let path = crate::store::test_dir::temp_db(&format!("guard-{tag}"));
    Sqlite::open(&path, &crate::store::insecure_test_key()).unwrap();
    path
}

fn gateway_on(path: &std::path::Path) -> Gateway<Sqlite> {
    let store = Sqlite::open(path, &crate::store::insecure_test_key()).unwrap();
    Gateway::new(store, FixedClock(1_000), AllowAll)
}

/// Run a guarded session on its own thread; its result arrives on the returned channel.
fn start(
    path: PathBuf,
    config: SessionConfig,
    (input, output, closer): (
        impl BufRead + Send + 'static,
        impl Write + Send + 'static,
        Option<Closer>,
    ),
) -> Receiver<std::io::Result<()>> {
    let (done, result) = channel();
    std::thread::spawn(move || {
        let gateway = gateway_on(&path);
        let who = alice();
        let ended = Session::new(&gateway, &who, &config).serve_guarded(input, output, closer);
        let _ = done.send(ended);
    });
    result
}

fn subscribe_line() -> Vec<u8> {
    format!(
        "{}\n",
        req("s", "subscribe", json!({"room":"r","from_seq":0}))
    )
    .into_bytes()
}

fn input_with(lines: Vec<Vec<u8>>) -> (Sender<Vec<u8>>, BufReader<ChanReader>) {
    let (tx, rx) = channel();
    for l in lines {
        tx.send(l).unwrap();
    }
    (
        tx,
        BufReader::new(ChanReader {
            rx,
            pending: Vec::new(),
        }),
    )
}

fn post_many(path: &std::path::Path, n: usize) {
    let g = gateway_on(path);
    for i in 0..n {
        g.post(&bob(), "r", &format!("m{i}")).unwrap();
    }
}

fn the_text(sink: &Arc<Mutex<Vec<u8>>>) -> String {
    String::from_utf8_lossy(&sink.lock().unwrap()).to_string()
}

#[test]
fn a_consumer_that_stops_reading_ends_the_session_with_a_refusal_instead_of_stalling_it() {
    let path = new_db("stuck");
    post_many(&path, 60);
    let (_keep_open, input) = input_with(vec![subscribe_line()]);
    let (gate, sink): (Gate, _) = (Gate::default(), Arc::new(Mutex::new(Vec::new())));
    let out = StuckOut {
        gate: gate.clone(),
        sink: sink.clone(),
    };
    let config = SessionConfig {
        outgoing: 2,
        stall: Duration::from_millis(300),
        ..fast()
    };
    let started = Instant::now();
    let result = start(path.to_path_buf(), config, (input, out, None));
    let ended = result
        .recv_timeout(Duration::from_secs(10))
        .expect("the session must end, not stall behind the blocked write");
    assert_eq!(ended.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(5));
    // The consumer comes back: it is told why, as the last thing it reads, and gets nothing after it.
    open_gate(&gate);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !the_text(&sink).contains("stream.slow_consumer") {
        assert!(Instant::now() < deadline, "no refusal: {}", the_text(&sink));
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(100));
    let text = the_text(&sink);
    assert!(
        text.lines()
            .last()
            .unwrap()
            .contains("stream.slow_consumer"),
        "the refusal is the last frame"
    );
}

#[test]
fn a_consumer_that_reads_slowly_but_keeps_reading_is_not_refused() {
    let path = new_db("slow");
    post_many(&path, 60);
    let (keep_open, input) = input_with(vec![subscribe_line()]);
    let out = SharedOut(Arc::new(Mutex::new(Vec::new())));
    let seen = out.0.clone();
    let config = SessionConfig {
        outgoing: 2,
        stall: Duration::from_secs(5),
        ..fast()
    };
    let result = start(path.to_path_buf(), config, (input, out, None));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !the_text(&seen).contains("caught_up") {
        assert!(Instant::now() < deadline, "never caught up");
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(keep_open);
    assert!(
        result
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .is_ok()
    );
    assert!(!the_text(&seen).contains("slow_consumer"));
}

#[test]
fn the_end_of_input_still_delivers_everything_queued_before_returning() {
    let path = new_db("flush");
    post_many(&path, 5);
    let (keep_open, input) = input_with(vec![subscribe_line()]);
    drop(keep_open);
    let out = SharedOut(Arc::new(Mutex::new(Vec::new())));
    let seen = out.0.clone();
    let result = start(path.to_path_buf(), fast(), (input, out, None));
    assert!(
        result
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .is_ok()
    );
    let text = the_text(&seen);
    assert!(
        text.contains("\"type\":\"hello\"") && text.contains("caught_up"),
        "{text}"
    );
    assert_eq!(text.matches("\"type\":\"message\"").count(), 5);
}

#[test]
fn the_reader_thread_ends_with_the_session_when_the_transport_can_close_the_input() {
    let path = new_db("closer");
    post_many(&path, 3);
    let (unblock, wake) = channel::<()>();
    let dropped = Arc::new(AtomicBool::new(false));
    let input = BufReader::new(NeverEnds {
        wake,
        dropped: dropped.clone(),
    });
    // The input never closes. The output is gone, so the session ends on its first write after the hello; the closer
    // is what lets the blocked reader go.
    let closer: Closer = Box::new(move || drop(unblock));
    let result = start(path.to_path_buf(), fast(), (input, BrokenOut, Some(closer)));
    let ended = result.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(ended.is_err(), "a dead output ends the session");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !dropped.load(Ordering::SeqCst) {
        assert!(
            Instant::now() < deadline,
            "the reader thread is still blocked in its read"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_session_whose_transport_has_no_closer_still_returns_while_the_input_stays_open() {
    let path = new_db("nocloser");
    post_many(&path, 3);
    let (_unblock, wake) = channel::<()>();
    let input = BufReader::new(NeverEnds {
        wake,
        dropped: Arc::new(AtomicBool::new(false)),
    });
    let result = start(path.to_path_buf(), fast(), (input, BrokenOut, None));
    let ended = result
        .recv_timeout(Duration::from_secs(10))
        .expect("the session must not wait for an input that never ends");
    assert!(ended.is_err());
}
