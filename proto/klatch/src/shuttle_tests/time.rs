//! The session loop over the shared gateway, with time (#666). The loop waits on its input for one poll period and then
//! looks at the store and the heartbeat clock; `sync::recv_within` is where Shuttle lets that period pass. The input is
//! held open and idle by the test, so only the period passing can make the session act.

use super::explore;
use crate::sync::{mpsc, thread};
use shuttle::sync::Mutex as ShMutex;
use std::sync::Arc;
use std::time::Duration;

/// An input the test feeds line by line and closes by dropping the sender: a blocked `read` is an idle open connection.
struct Feed {
    rx: mpsc::Receiver<Vec<u8>>,
    buf: Vec<u8>,
    at: usize,
}

impl std::io::Read for Feed {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        use std::io::BufRead;
        let got = self.fill_buf()?;
        let n = got.len().min(out.len());
        out[..n].copy_from_slice(&got[..n]);
        self.consume(n);
        Ok(n)
    }
}

impl std::io::BufRead for Feed {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        if self.at >= self.buf.len() {
            self.at = 0;
            self.buf = self.rx.recv().unwrap_or_default();
        }
        Ok(&self.buf[self.at..])
    }
    fn consume(&mut self, n: usize) {
        self.at += n;
    }
}

/// A session output the test can read while the session still runs.
#[derive(Clone, Default)]
struct Tap(Arc<ShMutex<Vec<u8>>>);

impl std::io::Write for Tap {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Tap {
    fn frames(&self) -> Vec<serde_json::Value> {
        let text = String::from_utf8(self.0.lock().unwrap().clone()).unwrap();
        text.lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
    /// Yield until a frame satisfying `want` is in the output.
    fn wait_for(&self, want: impl Fn(&serde_json::Value) -> bool) -> serde_json::Value {
        loop {
            if let Some(f) = self.frames().into_iter().find(|f| want(f)) {
                return f;
            }
            thread::yield_now();
        }
    }
}

fn request(id: &str, method: &str, params: &str) -> Vec<u8> {
    format!("{{\"v\":1,\"type\":\"request\",\"id\":\"{id}\",\"method\":\"{method}\",\"params\":{params}}}\n")
        .into_bytes()
}

/// A session on the shared gateway: the sender feeds its input, the tap shows its output.
fn open_session(
    gw: &crate::gateway::SharedGateway,
    member: &'static str,
    config: crate::session::SessionConfig,
) -> (mpsc::Sender<Vec<u8>>, Tap, thread::JoinHandle<()>) {
    let (feed_tx, feed_rx) = mpsc::channel();
    let tap = Tap::default();
    let (backend, out) = (gw.session_backend(), tap.clone());
    let session = thread::spawn(move || {
        let who = crate::domain::Principal::unattributed(member);
        let input = Feed {
            rx: feed_rx,
            buf: Vec::new(),
            at: 0,
        };
        crate::session::Session::over(&backend, &who, &config)
            .serve(input, out)
            .unwrap();
    });
    (feed_tx, tap, session)
}

fn scratch_gateway(
    tag: &str,
) -> (
    crate::gateway::SharedGateway,
    crate::store::test_dir::TempDb,
) {
    use crate::gateway::{AllowAll, Gateway, SharedGateway, SystemClock};
    let db = crate::store::test_dir::temp_db(tag);
    let path = db.to_path_buf();
    let gw = SharedGateway::spawn(move || {
        let store = crate::store::SqliteStore::open(&path, &crate::store::insecure_test_key())?;
        Ok(Gateway::new(store, SystemClock, AllowAll))
    })
    .unwrap();
    (gw, db)
}

fn instant_config() -> crate::session::SessionConfig {
    crate::session::SessionConfig {
        poll: Duration::ZERO,
        heartbeat: Duration::ZERO,
        ..Default::default()
    }
}

#[test]
fn an_idle_session_sends_a_heartbeat_and_then_ends_when_its_input_closes() {
    // No request ever arrives: the heartbeat can only come from the idle period passing. It must not need the input to
    // move, and closing the input afterwards must still end the session in every schedule.
    explore(|| {
        let (gw, _db) = scratch_gateway("shuttle-heartbeat");
        let (feed, tap, session) = open_session(&gw, "alice", instant_config());
        let beat = tap.wait_for(|f| f["type"] == "heartbeat");
        assert_eq!(beat["v"], 1);
        assert_eq!(tap.frames()[0]["type"], "hello", "hello comes first");
        drop(feed);
        session.join().expect("the session ended cleanly");
    });
}

#[test]
fn a_subscriber_with_an_idle_input_is_pushed_what_another_session_posts() {
    // Session A subscribes and then says nothing; session B posts through the same owner thread. A learns of the message
    // only by its poll period passing (its input stays open and silent), in every order of the two sessions and the owner.
    explore(|| {
        let (gw, _db) = scratch_gateway("shuttle-poll");
        let (a_feed, a_out, a) = open_session(&gw, "alice", instant_config());
        let (b_feed, b_out, b) = open_session(&gw, "bob", instant_config());
        for (feed, who) in [(&a_feed, "a"), (&b_feed, "b")] {
            feed.send(request(&format!("j{who}"), "join", "{\"room\":\"r\"}"))
                .unwrap();
        }
        a_out.wait_for(|f| f["id"] == "ja");
        a_feed
            .send(request("s", "subscribe", "{\"room\":\"r\",\"from_seq\":0}"))
            .unwrap();
        b_feed
            .send(request("p", "post", "{\"room\":\"r\",\"body\":\"hello\"}"))
            .unwrap();
        b_out.wait_for(|f| f["id"] == "p");
        let pushed = a_out.wait_for(|f| f["type"] == "message");
        assert_eq!(pushed["body"], "hello");
        assert_eq!(pushed["room"], "r");
        drop((a_feed, b_feed));
        a.join().unwrap();
        b.join().unwrap();
    });
}
