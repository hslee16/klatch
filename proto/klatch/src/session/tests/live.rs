use super::*;

use crate::store::SqliteStore as Sqlite;
use std::io::{BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub(super) struct ChanReader {
    pub(super) rx: Receiver<Vec<u8>>,
    pub(super) pending: Vec<u8>,
}
impl Read for ChanReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pending.is_empty() {
            match self.rx.recv() {
                Ok(bytes) => self.pending = bytes,
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.pending.len());
        buf[..n].copy_from_slice(&self.pending[..n]);
        self.pending.drain(..n);
        Ok(n)
    }
}

#[derive(Clone)]
pub(super) struct SharedOut(pub(super) Arc<Mutex<Vec<u8>>>);
impl Write for SharedOut {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(super) fn fast() -> SessionConfig {
    SessionConfig {
        poll: Duration::from_millis(5),
        heartbeat: Duration::from_secs(60),
        window: 100,
        batch: 64,
        ..SessionConfig::default()
    }
}

struct Live {
    tx: Option<Sender<Vec<u8>>>,
    out: SharedOut,
    handle: Option<std::thread::JoinHandle<std::io::Result<()>>>,
    path: std::path::PathBuf,
}

impl Live {
    fn start(who: Principal, config: SessionConfig) -> Self {
        Self::start_with(who, config, None)
    }

    fn start_with(who: Principal, config: SessionConfig, flaky: Option<Arc<Flaky>>) -> Self {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "klatch-live-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rooms.db");
        Sqlite::open(&path, &crate::store::insecure_test_key()).unwrap();
        let (tx, rx) = channel();
        let out = SharedOut(Arc::new(Mutex::new(Vec::new())));
        let (o2, p2) = (out.clone(), path.clone());
        let handle = std::thread::spawn(move || {
            let store = Sqlite::open(&p2, &crate::store::insecure_test_key()).unwrap();
            let input = BufReader::new(ChanReader {
                rx,
                pending: Vec::new(),
            });
            match flaky {
                Some(f) => {
                    let store = FlakyStore {
                        inner: store,
                        flags: f,
                    };
                    let gateway = Gateway::new(store, FixedClock(1_000), AllowAll);
                    Session::new(&gateway, &who, &config).serve(input, o2)
                }
                None => {
                    let gateway = Gateway::new(store, FixedClock(1_000), AllowAll);
                    Session::new(&gateway, &who, &config).serve(input, o2)
                }
            }
        });
        Live {
            tx: Some(tx),
            out,
            handle: Some(handle),
            path,
        }
    }

    fn send(&self, v: Value) {
        self.tx
            .as_ref()
            .unwrap()
            .send(format!("{v}\n").into_bytes())
            .unwrap();
    }

    fn frames(&self) -> Vec<ServerFrame> {
        frames_of(&self.out.0.lock().unwrap())
    }

    fn wait_for(&self, what: &str, pred: impl Fn(&[ServerFrame]) -> bool) -> Vec<ServerFrame> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let f = self.frames();
            if pred(&f) {
                return f;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; got {f:?}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A second connection to the same database, as another agent would have.
    fn other(&self) -> Gateway<Sqlite> {
        Gateway::new(
            Sqlite::open(&self.path, &crate::store::insecure_test_key()).unwrap(),
            FixedClock(2_000),
            AllowAll,
        )
    }

    fn end(mut self) {
        drop(self.tx.take());
        self.handle.take().unwrap().join().unwrap().unwrap();
        std::fs::remove_dir_all(self.path.parent().unwrap()).ok();
    }
}

fn caught_up(f: &[ServerFrame]) -> bool {
    f.iter().any(|x| matches!(x, ServerFrame::CaughtUp { .. }))
}

#[test]
fn a_live_message_is_pushed_after_caught_up() {
    let live = Live::start(alice(), fast());
    live.send(req("s", "subscribe", json!({"room":"r","from_seq":0})));
    live.wait_for("caught_up", caught_up);
    live.other().post(&bob(), "r", "hello live").unwrap();
    let f = live.wait_for("the live message", |f| !messages(f).is_empty());
    assert_eq!(
        messages(&f),
        vec![("r".to_string(), 1, "hello live".to_string())]
    );
    live.end();
}

#[test]
fn a_commit_by_another_connection_wakes_the_stream_long_before_the_poll_period() {
    // The poll period is ten seconds, so only the change token can deliver this within the wait_for limit (#148).
    let cfg = SessionConfig {
        poll: Duration::from_secs(10),
        ..fast()
    };
    let live = Live::start(alice(), cfg);
    live.send(req("s", "subscribe", json!({"room":"r","from_seq":0})));
    live.wait_for("the subscribe response", |f| {
        f.iter()
            .any(|x| matches!(x, ServerFrame::Response { id, .. } if id == "s"))
    });
    live.other().post(&bob(), "r", "woken").unwrap();
    let f = live.wait_for("the live message", |f| !messages(f).is_empty());
    assert_eq!(seqs(&f), vec![1]);
    live.end();
}

#[test]
fn an_idle_stream_sends_heartbeats() {
    let cfg = SessionConfig {
        heartbeat: Duration::from_millis(30),
        ..fast()
    };
    let live = Live::start(alice(), cfg);
    live.wait_for("a heartbeat", |f| {
        f.iter()
            .any(|x| matches!(x, ServerFrame::Heartbeat { ts, .. } if *ts == 1_000))
    });
    live.end();
}

#[test]
fn the_window_pauses_the_stream_until_the_client_acknowledges() {
    let cfg = SessionConfig {
        window: 2,
        ..fast()
    };
    let live = Live::start(alice(), cfg);
    let o = live.other();
    for i in 0..5 {
        o.post(&bob(), "r", &format!("m{i}")).unwrap();
    }
    live.send(req("s", "subscribe", json!({"room":"r","from_seq":0})));
    live.wait_for("two messages", |f| seqs(f).len() == 2);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(
        seqs(&live.frames()),
        vec![1, 2],
        "paused with a full window"
    );
    live.send(json!({"v":1,"type":"ack","room":"r","seq":2}));
    live.wait_for("two more", |f| seqs(f).len() == 4);
    live.send(json!({"v":1,"type":"ack","room":"r","seq":4}));
    live.wait_for("the last", |f| seqs(f).len() == 5);
    assert_eq!(seqs(&live.frames()), vec![1, 2, 3, 4, 5]);
    live.end();
}

#[test]
fn resubscribing_from_the_last_seq_resumes_without_duplicates() {
    let live = Live::start(alice(), fast());
    let o = live.other();
    o.post(&bob(), "r", "a").unwrap();
    o.post(&bob(), "r", "b").unwrap();
    live.send(req("s1", "subscribe", json!({"room":"r","from_seq":0})));
    live.wait_for("first replay", |f| seqs(f).len() == 2 && caught_up(f));
    live.send(req("s2", "subscribe", json!({"room":"r","from_seq":2})));
    live.wait_for("second response", |f| {
        f.iter()
            .any(|x| matches!(x, ServerFrame::Response { id, .. } if id == "s2"))
    });
    o.post(&bob(), "r", "c").unwrap();
    let f = live.wait_for("the new message", |f| seqs(f).len() == 3);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(seqs(&live.frames()), vec![1, 2, 3], "no duplicates, {f:?}");
    live.end();
}

#[test]
fn unsubscribe_stops_the_stream() {
    let live = Live::start(alice(), fast());
    live.send(req("s", "subscribe", json!({"room":"r","from_seq":0})));
    live.wait_for("caught_up", caught_up);
    live.send(req("u", "unsubscribe", json!({"room":"r"})));
    live.wait_for("the unsubscribe response", |f| {
        f.iter()
            .any(|x| matches!(x, ServerFrame::Response { id, ok: true, .. } if id == "u"))
    });
    live.other().post(&bob(), "r", "unheard").unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(messages(&live.frames()).is_empty());
    live.end();
}

#[test]
fn two_rooms_stream_independently() {
    let live = Live::start(alice(), fast());
    live.send(req("a", "subscribe", json!({"room":"one","from_seq":0})));
    live.send(req("b", "subscribe", json!({"room":"two","from_seq":0})));
    live.wait_for("both caught up", |f| {
        f.iter()
            .filter(|x| matches!(x, ServerFrame::CaughtUp { .. }))
            .count()
            == 2
    });
    let o = live.other();
    o.post(&bob(), "two", "in two").unwrap();
    o.post(&bob(), "one", "in one").unwrap();
    let f = live.wait_for("both messages", |f| messages(f).len() == 2);
    let mut got = messages(&f);
    got.sort();
    assert_eq!(
        got,
        vec![
            ("one".to_string(), 1, "in one".to_string()),
            ("two".to_string(), 1, "in two".to_string())
        ]
    );
    live.end();
}

#[test]
fn an_ack_frame_moves_the_cursor_while_the_session_is_live() {
    let live = Live::start(alice(), fast());
    let o = live.other();
    o.join(&alice(), "r").unwrap();
    for i in 0..3 {
        o.post(&bob(), "r", &format!("m{i}")).unwrap();
    }
    live.send(json!({"v":1,"type":"ack","room":"r","seq":3}));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !o.read(&alice(), "r", false).unwrap().is_empty() {
        assert!(
            Instant::now() < deadline,
            "the ack never advanced the cursor"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    live.end();
}

#[test]
fn the_session_ends_when_the_input_closes_even_with_subscriptions() {
    let live = Live::start(alice(), fast());
    live.send(req("s", "subscribe", json!({"room":"r","from_seq":0})));
    live.wait_for("caught_up", caught_up);
    live.end(); // joins the thread: a hang here is the failure
}

struct Flaky {
    fail: AtomicBool,
    busy: AtomicBool,
}
struct FlakyStore {
    inner: Sqlite,
    flags: Arc<Flaky>,
}
impl FlakyStore {
    fn gate(&self) -> Result<()> {
        if self.flags.busy.load(Ordering::SeqCst) {
            return Err(Error::Busy("locked".into()));
        }
        if self.flags.fail.load(Ordering::SeqCst) {
            return Err(Error::Backend("disk on fire".into()));
        }
        Ok(())
    }
}
impl MessageStore for FlakyStore {
    fn join(&self, r: &str, m: &str, n: i64) -> Result<()> {
        self.inner.join(r, m, n)
    }

    fn post_as(&self, message: &NewMessage, now: i64) -> Result<Posted> {
        self.inner.post_as(message, now)
    }

    fn read(&self, at: Seat, advance: bool, now: i64) -> Result<Vec<Message>> {
        self.inner.read(at, advance, now)
    }

    fn tail(&self, r: &str, a: i64, l: usize) -> Result<Vec<Message>> {
        self.gate()?;
        self.inner.tail(r, a, l)
    }

    fn head(&self, r: &str) -> Result<i64> {
        self.inner.head(r)
    }

    fn advance_cursor(&self, at: Seat, seq: Seq, now: i64) -> Result<()> {
        self.inner.advance_cursor(at, seq, now)
    }
}
impl MemberStore for FlakyStore {
    fn members(&self, r: &str) -> Result<Vec<MemberInfo>> {
        self.inner.members(r)
    }

    fn rooms_of(&self, m: &str) -> Result<Vec<String>> {
        self.inner.rooms_of(m)
    }
}
impl ClaimStore for FlakyStore {
    fn claim(&self, request: &ClaimRequest, now: i64) -> Result<()> {
        self.inner.claim(request, now)
    }

    fn release(&self, r: &str, o: &str, p: &str) -> Result<()> {
        self.inner.release(r, o, p)
    }

    fn claims(&self, r: &str, n: i64) -> Result<Vec<Claim>> {
        self.inner.claims(r, n)
    }
}
impl AskStore for FlakyStore {}
impl ReviewStore for FlakyStore {}
impl CredentialStore for FlakyStore {}
impl PolicyStore for FlakyStore {}
impl RoomStore for FlakyStore {}

#[test]
fn a_busy_store_is_retried_silently_and_the_stream_catches_up() {
    let flags = Arc::new(Flaky {
        fail: AtomicBool::new(false),
        busy: AtomicBool::new(false),
    });
    let live = Live::start_with(alice(), fast(), Some(flags.clone()));
    live.send(req("s", "subscribe", json!({"room":"r","from_seq":0})));
    live.wait_for("caught_up", caught_up);
    flags.busy.store(true, Ordering::SeqCst);
    live.other().post(&bob(), "r", "during the lock").unwrap();
    std::thread::sleep(Duration::from_millis(80));
    assert!(messages(&live.frames()).is_empty());
    assert!(
        !live
            .frames()
            .iter()
            .any(|x| matches!(x, ServerFrame::Refusal { .. })),
        "busy is not reported"
    );
    flags.busy.store(false, Ordering::SeqCst);
    live.wait_for("the message after the lock", |f| messages(f).len() == 1);
    live.end();
}

#[test]
fn a_store_failure_mid_stream_refuses_drops_the_subscription_and_allows_resubscribing() {
    let flags = Arc::new(Flaky {
        fail: AtomicBool::new(false),
        busy: AtomicBool::new(false),
    });
    let live = Live::start_with(alice(), fast(), Some(flags.clone()));
    live.send(req("s", "subscribe", json!({"room":"r","from_seq":0})));
    live.wait_for("caught_up", caught_up);
    flags.fail.store(true, Ordering::SeqCst);
    live.other().post(&bob(), "r", "x").unwrap();
    live.wait_for("a refusal", |f| {
        f.iter()
            .any(|x| matches!(x, ServerFrame::Refusal { rule, .. } if rule == "stream.backend"))
    });
    flags.fail.store(false, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(80));
    assert!(
        messages(&live.frames()).is_empty(),
        "the subscription was dropped"
    );
    live.send(req("again", "subscribe", json!({"room":"r","from_seq":0})));
    live.wait_for("the replay after resubscribing", |f| messages(f).len() == 1);
    live.end();
}
