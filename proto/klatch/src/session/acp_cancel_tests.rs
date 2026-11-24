//! `session/cancel` while a prompt is being handled (#1042). The adapter reads its input on a helper thread, so a cancel
//! that arrives while a look is running is seen at once; the prompt then ends with `stopReason` `cancelled` and no
//! answer. The tests give the agent a backend whose look waits until the test lets it go, and a client that sends
//! each line only when the test says, so every order of events is made by the test, not by timing. Every wait in
//! the harness has a limit, so a broken build fails an assertion instead of hanging.

use super::acp::AcpServer;
use super::acp_tests::{gateway, init, new_session, prompt};
use super::{Backend, Session, SessionConfig};
use crate::domain::{Principal, Result};
use crate::gateway::{Batch, Gateway, Subscription};
use crate::protocol::ServerFrame;
use crate::session::methods::Call;
use crate::store::SqliteStore;
use serde_json::{Value, json};
use std::io::{self, BufRead, Cursor, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

/// How long a test waits for anything before it calls the run broken.
const WAIT: Duration = Duration::from_secs(5);

/// A gateway whose plain requests (the looks) wait for the test: each call says it started, then waits for a release.
struct SlowLooks {
    inner: Gateway<SqliteStore>,
    started: Sender<()>,
    release: Receiver<()>,
    calls: Arc<AtomicUsize>,
}

impl Backend for SlowLooks {
    fn now(&self) -> i64 {
        self.inner.now()
    }
    fn change_token(&self) -> Option<u64> {
        Backend::change_token(&self.inner)
    }
    fn head(&self, who: &Principal, room: &str) -> Result<i64> {
        Backend::head(&self.inner, who, room)
    }
    fn ack(&self, who: &Principal, room: &str, seq: i64) -> Result<()> {
        Backend::ack(&self.inner, who, room, seq)
    }
    fn poll(&self, sub: &mut Subscription, who: &Principal, max: usize) -> Result<Batch> {
        Backend::poll(&self.inner, sub, who, max)
    }
    fn handle(&self, who: &Principal, call: Call) -> Vec<ServerFrame> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let _ = self.started.send(());
        // Released, or the test is gone (the sender dropped): either way go on, so a failed test does not hang here.
        let _ = self.release.recv_timeout(WAIT);
        Backend::handle(&self.inner, who, call)
    }
    fn verify_credential(&self, member: &str, token: &str) -> Result<Principal> {
        Backend::verify_credential(&self.inner, member, token)
    }
    fn note_member_fixed(&self, who: &Principal) {
        Backend::note_member_fixed(&self.inner, who);
    }
}

/// The agent's input: bytes the test sends, one chunk per line. Every time it has handed over all it had and is asked
/// for more, it says how many chunks it has handed over, which is how the test knows the reader has finished a line.
struct ChanInput {
    rx: Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
    given: usize,
    wants: Sender<usize>,
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
            let _ = self.wants.send(self.given);
            match self.rx.recv_timeout(WAIT) {
                Ok(bytes) => {
                    self.buf = bytes;
                    self.given += 1;
                }
                Err(RecvTimeoutError::Disconnected) => self.buf.clear(),
                Err(RecvTimeoutError::Timeout) => {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "no input came"));
                }
            }
            self.pos = 0;
        }
        Ok(&self.buf[self.pos..])
    }
    fn consume(&mut self, n: usize) {
        self.pos += n;
    }
}

/// The agent's output: each finished line goes to the test.
struct LineSink {
    lines: Sender<String>,
    partial: Vec<u8>,
}

impl Write for LineSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        for b in bytes {
            if *b == b'\n' {
                let line = String::from_utf8_lossy(&self.partial).into_owned();
                let _ = self.lines.send(line);
                self.partial.clear();
            } else {
                self.partial.push(*b);
            }
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The test's side: sends lines, waits for the reader and the look, lets the look go, reads what the agent printed.
struct Client {
    input: Option<Sender<Vec<u8>>>,
    wants: Receiver<usize>,
    sent: usize,
    started: Receiver<()>,
    release: Sender<()>,
    lines: Receiver<String>,
    seen: Vec<Value>,
}

type Step = std::result::Result<(), String>;

impl Client {
    fn send(&mut self, message: &Value) {
        self.send_bytes(format!("{message}\n").into_bytes());
    }
    fn send_bytes(&mut self, bytes: Vec<u8>) {
        self.input
            .as_ref()
            .expect("input already closed")
            .send(bytes)
            .expect("the agent stopped reading");
        self.sent += 1;
    }
    /// Wait until the reader has read and dealt with everything sent so far.
    fn reader_caught_up(&self) -> Step {
        let end = Instant::now() + WAIT;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            match self.wants.recv_timeout(left) {
                Ok(n) if n >= self.sent => return Ok(()),
                Ok(_) => {}
                Err(_) => return Err("the reader never caught up with the input".into()),
            }
        }
    }
    /// Wait until a look has started (it is now waiting for `release`).
    fn look_started(&self) -> Step {
        self.started
            .recv_timeout(WAIT)
            .map_err(|_| "no look started".to_string())
    }
    fn release(&self) {
        let _ = self.release.send(());
    }
    /// Wait for the agent to print the response with this id.
    fn response(&mut self, id: u64) -> Step {
        let end = Instant::now() + WAIT;
        while !self.seen.iter().any(|m| m["id"] == id) {
            let left = end.saturating_duration_since(Instant::now());
            let line = self
                .lines
                .recv_timeout(left)
                .map_err(|_| format!("no response {id}"))?;
            self.seen
                .push(serde_json::from_str(&line).expect("a JSON line"));
        }
        Ok(())
    }
    fn end_input(&mut self) {
        self.input = None;
    }
}

struct Run {
    served: io::Result<()>,
    script: Step,
    out: Vec<Value>,
    looks: usize,
    took: Duration,
}

/// Serve the agent as `member` over the slow looks, with `script` playing the client on its own thread.
fn run(script: impl FnOnce(&mut Client) -> Step + Send + 'static) -> Run {
    let (started_tx, started) = channel();
    let (release, release_rx) = channel();
    let (input, input_rx) = channel();
    let (wants_tx, wants) = channel();
    let (lines_tx, lines) = channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let backend = SlowLooks {
        inner: gateway(),
        started: started_tx,
        release: release_rx,
        calls: calls.clone(),
    };
    let mut client = Client {
        input: Some(input),
        wants,
        sent: 0,
        started,
        release,
        lines,
        seen: Vec::new(),
    };
    let controller = std::thread::spawn(move || {
        let result = script(&mut client);
        client.end_input();
        (result, client.lines, client.seen)
    });
    let who = Principal::unattributed("lib");
    let config = SessionConfig::default();
    let session = Session::over(&backend, &who, &config);
    let reader = ChanInput {
        rx: input_rx,
        buf: Vec::new(),
        pos: 0,
        given: 0,
        wants: wants_tx,
    };
    let begun = Instant::now();
    let mut sink = LineSink {
        lines: lines_tx,
        partial: Vec::new(),
    };
    let served = AcpServer::new(&session).serve(reader, &mut sink);
    let took = begun.elapsed();
    drop(sink);
    let (script, lines, mut out) = controller.join().expect("the script panicked");
    out.extend(
        lines
            .try_iter()
            .map(|l| serde_json::from_str::<Value>(&l).unwrap()),
    );
    Run {
        served,
        script,
        out,
        looks: calls.load(Ordering::SeqCst),
        took,
    }
}

fn cancel(session: &str) -> Value {
    json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":session}})
}

fn ok(run: &Run) {
    assert_eq!(
        run.script,
        Ok(()),
        "the script failed; output {:?}",
        run.out
    );
    assert!(run.served.is_ok(), "serve failed: {:?}", run.served);
}

fn updates(out: &[Value]) -> usize {
    out.iter()
        .filter(|m| m["method"] == "session/update")
        .count()
}

fn stop_of(out: &[Value], id: u64) -> Value {
    out.iter()
        .find(|m| m["id"] == id)
        .unwrap_or_else(|| panic!("no response {id} in {out:?}"))["result"]["stopReason"]
        .clone()
}

#[test]
fn a_cancel_that_arrives_while_the_look_runs_ends_the_prompt_cancelled_with_no_answer() {
    let r = run(|c| {
        c.send(&init());
        c.send(&new_session());
        c.send(&prompt(3, "read r"));
        c.look_started()?;
        c.send(&cancel("klatch-librarian-1"));
        // The cancel is read while the look is still running; only then does the look go on.
        c.reader_caught_up()?;
        c.release();
        Ok(())
    });
    ok(&r);
    assert_eq!(stop_of(&r.out, 3), "cancelled", "{:?}", r.out);
    assert_eq!(updates(&r.out), 0, "an answer was sent: {:?}", r.out);
    assert_eq!(r.out.len(), 3, "{:?}", r.out);
    assert!(r.took < WAIT / 2, "took {:?}", r.took);
}

#[test]
fn a_cancel_with_no_prompt_running_is_ignored_and_the_next_prompt_is_answered() {
    let r = run(|c| {
        c.send(&new_session());
        c.send(&prompt(3, "read r"));
        c.look_started()?;
        c.release();
        c.response(3)?;
        // The prompt is over; a cancel now is for nothing.
        c.send(&cancel("klatch-librarian-1"));
        c.reader_caught_up()?;
        c.send(&prompt(4, "read r"));
        c.look_started()?;
        c.release();
        Ok(())
    });
    ok(&r);
    assert_eq!(stop_of(&r.out, 3), "end_turn", "{:?}", r.out);
    assert_eq!(stop_of(&r.out, 4), "end_turn", "{:?}", r.out);
    assert_eq!(updates(&r.out), 2, "{:?}", r.out);
    assert!(
        r.out.iter().all(|m| m.get("error").is_none()),
        "{:?}",
        r.out
    );
}

#[test]
fn a_cancel_for_another_session_id_is_ignored() {
    let r = run(|c| {
        c.send(&new_session());
        c.send(&prompt(3, "read r"));
        c.look_started()?;
        c.send(&cancel("klatch-librarian-9"));
        c.send(&cancel("nonsense"));
        c.reader_caught_up()?;
        c.release();
        Ok(())
    });
    ok(&r);
    assert_eq!(stop_of(&r.out, 3), "end_turn", "{:?}", r.out);
    assert_eq!(updates(&r.out), 1, "{:?}", r.out);
}

#[test]
fn a_cancel_that_names_no_session_or_is_a_request_does_not_cancel() {
    let bare = json!({"jsonrpc":"2.0","method":"session/cancel","params":{}});
    let with_id = json!({"jsonrpc":"2.0","id":7,"method":"session/cancel","params":{"sessionId":"klatch-librarian-1"}});
    let r = run(move |c| {
        c.send(&new_session());
        c.send(&prompt(3, "read r"));
        c.look_started()?;
        c.send(&bare);
        c.send(&with_id);
        c.reader_caught_up()?;
        c.release();
        Ok(())
    });
    ok(&r);
    assert_eq!(stop_of(&r.out, 3), "end_turn", "{:?}", r.out);
    // A cancel that is a request (it has an id) is not the notification; it answers as it did before: no such method.
    let answer = r.out.iter().find(|m| m["id"] == 7).expect("an answer to 7");
    assert_eq!(answer["error"]["code"], -32601, "{:?}", r.out);
}

#[test]
fn two_prompts_in_sequence_are_both_answered_in_order() {
    let r = run(|c| {
        c.send(&new_session());
        c.send(&prompt(3, "read r"));
        c.send(&prompt(4, "who r"));
        c.look_started()?;
        c.release();
        c.look_started()?;
        c.release();
        Ok(())
    });
    ok(&r);
    let order: Vec<String> = r
        .out
        .iter()
        .map(|m| {
            if m["method"] == "session/update" {
                "update".to_string()
            } else {
                m["id"].to_string()
            }
        })
        .collect();
    assert_eq!(order, ["2", "update", "3", "update", "4"], "{:?}", r.out);
    assert_eq!(stop_of(&r.out, 3), "end_turn");
    assert_eq!(stop_of(&r.out, 4), "end_turn");
}

#[test]
fn a_cancelled_prompt_does_not_cancel_the_next_one() {
    let r = run(|c| {
        c.send(&new_session());
        c.send(&prompt(3, "read r"));
        c.look_started()?;
        c.send(&cancel("klatch-librarian-1"));
        c.reader_caught_up()?;
        c.release();
        c.response(3)?;
        c.send(&prompt(4, "read r"));
        c.look_started()?;
        c.release();
        Ok(())
    });
    ok(&r);
    assert_eq!(stop_of(&r.out, 3), "cancelled", "{:?}", r.out);
    assert_eq!(stop_of(&r.out, 4), "end_turn", "{:?}", r.out);
    assert_eq!(updates(&r.out), 1, "{:?}", r.out);
}

#[test]
fn a_prompt_that_was_read_but_not_started_when_its_cancel_came_never_runs_its_look() {
    let r = run(|c| {
        c.send(&new_session());
        c.send(&prompt(3, "read r"));
        c.look_started()?;
        c.send(&prompt(4, "read r"));
        c.send(&cancel("klatch-librarian-1"));
        c.reader_caught_up()?;
        c.release();
        Ok(())
    });
    ok(&r);
    assert_eq!(stop_of(&r.out, 3), "cancelled", "{:?}", r.out);
    assert_eq!(stop_of(&r.out, 4), "cancelled", "{:?}", r.out);
    assert_eq!(r.looks, 1, "the queued prompt still ran its look");
    assert_eq!(updates(&r.out), 0, "{:?}", r.out);
}

#[test]
fn the_end_of_input_mid_prompt_lets_the_prompt_finish_and_ends_the_session_cleanly() {
    let r = run(|c| {
        c.send(&new_session());
        c.send(&prompt(3, "read r"));
        c.look_started()?;
        c.end_input();
        c.release();
        Ok(())
    });
    ok(&r);
    assert_eq!(stop_of(&r.out, 3), "end_turn", "{:?}", r.out);
    assert_eq!(updates(&r.out), 1, "{:?}", r.out);
}

#[test]
fn a_malformed_line_mid_prompt_is_answered_after_the_prompt_as_before() {
    let r = run(|c| {
        c.send(&new_session());
        c.send(&prompt(3, "read r"));
        c.look_started()?;
        c.send_bytes(b"this is not json\n".to_vec());
        c.reader_caught_up()?;
        c.release();
        Ok(())
    });
    ok(&r);
    // The same lines through the unchanged order: the prompt's update and response, then the parse error with a null id.
    let lines = [
        new_session(),
        prompt(3, "read r"),
        Value::String("this is not json".into()),
    ];
    let text: String = lines
        .iter()
        .map(|l| match l {
            Value::String(s) => format!("{s}\n"),
            other => format!("{other}\n"),
        })
        .collect();
    let g = gateway();
    let who = Principal::unattributed("lib");
    let config = SessionConfig::default();
    let session = Session::new(&g, &who, &config);
    let mut plain = Vec::new();
    AcpServer::new(&session)
        .serve(Cursor::new(text), &mut plain)
        .unwrap();
    let expected: Vec<Value> = String::from_utf8(plain)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    // Message ids and times differ between two gateways, so compare the shape: the same messages in the same order, the
    // same responses and errors exactly.
    assert_eq!(r.out.len(), expected.len());
    for (got, want) in r.out.iter().zip(&expected) {
        if want["method"] == "session/update" {
            assert_eq!(got["method"], "session/update");
        } else {
            assert_eq!(got, want);
        }
    }
    assert_eq!(r.out[3]["error"]["code"], -32700);
    assert_eq!(r.out[3]["id"], Value::Null);
}

#[test]
fn input_that_cannot_be_read_mid_prompt_ends_the_session_with_the_read_error_after_the_prompt() {
    let r = run(|c| {
        c.send(&new_session());
        c.send(&prompt(3, "read r"));
        c.look_started()?;
        c.send_bytes(vec![0xff, 0xfe, b'\n']);
        c.release();
        Ok(())
    });
    assert_eq!(r.script, Ok(()), "{:?}", r.out);
    let err = r
        .served
        .expect_err("the session should end with the read error");
    assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{err}");
    // The prompt in flight was still answered before the session ended.
    assert_eq!(stop_of(&r.out, 3), "end_turn", "{:?}", r.out);
}

/// The input is dropped on every way out, so no thread is left holding it. A session that ended because the output
/// failed does not join the reader (it could be blocked in a read nothing will end), so that one is waited for.
#[test]
fn the_input_is_released_on_every_way_out() {
    let (dropped, drops) = channel::<()>();
    struct Counted {
        inner: Cursor<Vec<u8>>,
        dropped: Sender<()>,
    }
    impl Drop for Counted {
        fn drop(&mut self) {
            let _ = self.dropped.send(());
        }
    }
    impl Read for Counted {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            self.inner.read(b)
        }
    }
    impl BufRead for Counted {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            self.inner.fill_buf()
        }
        fn consume(&mut self, n: usize) {
            self.inner.consume(n)
        }
    }
    // A write that fails, an input that ends, and an input with junk: the input is dropped on each.
    let g = gateway();
    let who = Principal::unattributed("lib");
    let config = SessionConfig::default();
    let session = Session::new(&g, &who, &config);
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "gone"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let lines = format!("{}\n{}\n", init(), init());
    for (text, broken) in [
        (lines.clone(), true),
        (lines.clone(), false),
        ("\u{0}\n".to_string(), false),
    ] {
        let input = Counted {
            inner: Cursor::new(text.into_bytes()),
            dropped: dropped.clone(),
        };
        let _ = if broken {
            AcpServer::new(&session).serve(input, Broken)
        } else {
            AcpServer::new(&session).serve(input, Vec::new())
        };
    }
    for _ in 0..3 {
        drops.recv_timeout(WAIT).expect("an input was still held");
    }
}
