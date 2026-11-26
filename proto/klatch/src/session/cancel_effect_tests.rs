//! Only a pure look may be cancelled (#1562). A request with an effect (a post, a claim, a read that moves the cursor)
//! must never be reported `Cancelled`: the agent would be told "cancelled" after the effect happened. These tests hold
//! the store's write lock from a second connection so the owner blocks inside the request after it has passed every
//! check, then cancel, as the reviewer reproduced it. Also: the ACP hook key names the session, so two prompts with the
//! same JSON-RPC id in different sessions do not collide, and a prompt for a session that does not exist is not recorded.

use super::cancel_hook_tests::{
    ChanInput, HeldOwner, LineSink, PROMPT, WAIT, is_cancelled, shared, wait_in_flight,
};
use super::{Backend, CancelHook, Outcome, Session, SessionConfig};
use crate::domain::{Principal, Result};
use crate::gateway::{AllowAll, Batch, Gateway, SharedGateway, Subscription, SystemClock};
use crate::protocol::ServerFrame;
use crate::session::acp_reader;
use crate::session::methods::Call;
use crate::store::SqliteStore;
use crate::store::test_dir::{TempDb, temp_db};
use serde_json::{Value, json};
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A shared gateway whose store waits up to 3 s for the write lock, so a held lock blocks the owner inside a write.
fn locked_shared(tag: &str) -> (SharedGateway, TempDb) {
    let db = temp_db(tag);
    let path = db.to_path_buf();
    let gw = SharedGateway::spawn(move || {
        let store = SqliteStore::open_with_timeout(
            &path,
            &crate::store::insecure_test_key(),
            Duration::from_secs(3),
        )?;
        Ok(Gateway::new(store, SystemClock, AllowAll))
    })
    .unwrap();
    (gw, db)
}

fn holder(db: &TempDb) -> rusqlite::Connection {
    let c = rusqlite::Connection::open(db).unwrap();
    c.pragma_update(
        None,
        "key",
        crate::store::insecure_test_key()
            .sqlcipher_pragma()
            .as_str(),
    )
    .unwrap();
    c
}

fn call(id: &str, method: &str, value: Value) -> Call {
    Call {
        id: id.into(),
        method: method.into(),
        value,
    }
}

#[test]
fn a_request_with_an_effect_is_never_reported_cancelled_when_a_cancel_arrives_while_it_is_blocked()
{
    let cases = [
        ("post", json!({"room": "r", "body": "marker-effect"})),
        ("claim", json!({"room": "r", "target": "marker-effect"})),
        ("read", json!({"room": "r"})),
        ("read", json!({"room": "r", "peek": false})),
    ];
    for (n, (method, value)) in cases.into_iter().enumerate() {
        let (gw, db) = locked_shared(&format!("cancel-effect-{n}"));
        let backend = Arc::new(gw.session_backend());
        let who = Principal::unattributed("lib");
        backend.handle(&who, call("j", "join", json!({"room": "r"})));
        let lock = holder(&db);
        lock.execute_batch("BEGIN IMMEDIATE").unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let (b2, w2) = (backend.clone(), who.clone());
        let request = call("e", method, value);
        std::thread::spawn(move || {
            let _ = tx.send(b2.handle_cancellable(&w2, request, None));
        });
        let mut finished_early = false;
        // The owner is now stuck behind the lock inside the request. Cancel for as long as it stays there.
        let end = Instant::now() + Duration::from_millis(400);
        while Instant::now() < end {
            assert!(
                !backend.canceller().cancel("e"),
                "{method}: a cancel claimed to cancel a request with an effect"
            );
            // A request that fails by itself (a parameter it does not accept) ends early and normally: fine, but
            // never as cancelled.
            if let Ok(out) = rx.recv_timeout(Duration::from_millis(10)) {
                assert!(
                    matches!(out, Outcome::Done(_)),
                    "{method}: reported cancelled"
                );
                finished_early = true;
                break;
            }
        }
        lock.execute_batch("ROLLBACK").unwrap();
        assert!(
            method != "post" || !finished_early,
            "the post never blocked, so this proves nothing"
        );
        if !finished_early {
            let out = rx.recv_timeout(WAIT).expect("the request never finished");
            assert!(
                matches!(out, Outcome::Done(_)),
                "{method}: reported cancelled"
            );
        }
        if method == "post" {
            let h = format!(
                "{:?}",
                backend.handle(&who, call("h", "history", json!({"room": "r", "last": 5})))
            );
            assert!(h.contains("marker-effect"), "the post did not land: {h}");
        }
    }
}

#[test]
fn the_pure_looks_still_cancel() {
    let looks = [
        ("who", json!({"room": "r"})),
        ("claims", json!({"room": "r"})),
        ("history", json!({"room": "r", "last": 3})),
        ("read", json!({"room": "r", "peek": true})),
    ];
    for (n, (method, value)) in looks.into_iter().enumerate() {
        let (gw, _db) = shared(&format!("cancel-pure-{n}"));
        let backend = Arc::new(gw.session_backend());
        let owner = HeldOwner::hold(&gw);
        let (tx, rx) = std::sync::mpsc::channel();
        let b2 = backend.clone();
        let request = call("l", method, value);
        std::thread::spawn(move || {
            let who = Principal::unattributed("lib");
            let _ = tx.send(b2.handle_cancellable(&who, request, None));
        });
        wait_in_flight(&backend, 1);
        assert!(
            backend.canceller().cancel("l"),
            "{method} was not cancellable"
        );
        assert!(is_cancelled(&rx.recv_timeout(PROMPT).unwrap()), "{method}");
        owner.release();
    }
}

/// A backend that cancels every cancellable request, to see what the session hands it.
struct AlwaysCancels(Gateway<SqliteStore>);

impl Backend for AlwaysCancels {
    fn now(&self) -> i64 {
        self.0.now()
    }
    fn change_token(&self) -> Option<u64> {
        Backend::change_token(&self.0)
    }
    fn head(&self, who: &Principal, room: &str) -> Result<i64> {
        Backend::head(&self.0, who, room)
    }
    fn ack(&self, who: &Principal, room: &str, seq: i64) -> Result<()> {
        Backend::ack(&self.0, who, room, seq)
    }
    fn poll(&self, sub: &mut Subscription, who: &Principal, max: usize) -> Result<Batch> {
        Backend::poll(&self.0, sub, who, max)
    }
    fn handle(&self, who: &Principal, call: Call) -> Vec<ServerFrame> {
        Backend::handle(&self.0, who, call)
    }
    fn handle_cancellable(
        &self,
        _who: &Principal,
        _call: Call,
        _harness: Option<&dyn crate::harness::Harness>,
    ) -> Outcome {
        Outcome::Cancelled
    }
    fn verify_credential(&self, member: &str, token: &str) -> Result<Principal> {
        Backend::verify_credential(&self.0, member, token)
    }
    fn note_member_fixed(&self, who: &Principal) {
        Backend::note_member_fixed(&self.0, who);
    }
}

#[test]
fn the_session_hands_only_pure_looks_to_the_cancellable_path() {
    let g = Gateway::new(
        SqliteStore::open_in_memory().unwrap(),
        SystemClock,
        AllowAll,
    );
    let backend = AlwaysCancels(g);
    let who = Principal::unattributed("lib");
    let config = SessionConfig::default();
    let session = Session::over(&backend, &who, &config);
    session.call("join", json!({"room": "r"}));
    for (method, value) in [
        ("post", json!({"room": "r", "body": "x"})),
        ("claim", json!({"room": "r", "target": "x"})),
        ("read", json!({"room": "r"})),
        ("read", json!({"room": "r", "peek": false})),
        ("read", json!({"room": "r", "peek": "true"})),
    ] {
        assert!(
            matches!(
                session.call_cancellable("c", method, value),
                Outcome::Done(_)
            ),
            "{method} reached the cancellable path"
        );
    }
    for (method, value) in [
        ("who", json!({"room": "r"})),
        ("claims", json!({"room": "r"})),
        ("history", json!({"room": "r", "last": 1})),
        ("read", json!({"room": "r", "peek": true})),
    ] {
        assert!(
            is_cancelled(&session.call_cancellable("c", method, value)),
            "{method} did not reach the cancellable path"
        );
    }
}

/// Records every call id it is asked to cancel.
#[derive(Default)]
struct Recorder(Mutex<Vec<String>>);

impl CancelHook for Recorder {
    fn cancel(&self, call_id: &str) -> bool {
        self.0.lock().unwrap().push(call_id.to_string());
        true
    }
}

/// The hook key the design names: the session and the JSON-RPC id.
fn key(session: &str, id: u64) -> String {
    format!("acp-{session}-{id}")
}

fn lines(items: &[Value]) -> Cursor<Vec<u8>> {
    Cursor::new(
        items
            .iter()
            .map(|v| format!("{v}\n"))
            .collect::<String>()
            .into_bytes(),
    )
}

fn req(id: u64, method: &str, session: Option<&str>) -> Value {
    let params = session.map_or_else(|| json!({}), |s| json!({"sessionId": s, "prompt": []}));
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

#[test]
fn the_hook_key_names_the_session_and_a_prompt_for_an_unknown_session_is_not_recorded() {
    let hook = Arc::new(Recorder::default());
    let input = lines(&[
        req(1, "session/new", None),
        req(2, "session/new", None),
        // The same JSON-RPC id 3 in two sessions, and one prompt for a session that does not exist.
        req(3, "session/prompt", Some("klatch-librarian-1")),
        req(3, "session/prompt", Some("klatch-librarian-2")),
        req(4, "session/prompt", Some("klatch-librarian-9")),
        req(5, "session/prompt", Some("klatch-librarian-0")),
        json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":"klatch-librarian-1"}}),
        json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":"klatch-librarian-9"}}),
    ]);
    let (rx, reader) = acp_reader::spawn_with(input, hook.clone());
    let got: Vec<_> = std::iter::from_fn(|| rx.recv().ok())
        .map(|l| l.unwrap())
        .collect();
    reader.join().unwrap();
    assert_eq!(
        *hook.0.lock().unwrap(),
        vec![key("klatch-librarian-1", 3)],
        "only session 1's prompt is cancelled, under a key that names its session"
    );
    assert_ne!(key("klatch-librarian-1", 3), key("klatch-librarian-2", 3),);
    let flag = |i: usize| {
        got[i]
            .1
            .as_ref()
            .map(|f| f.load(std::sync::atomic::Ordering::SeqCst))
    };
    assert_eq!(flag(2), Some(true));
    assert_eq!(flag(3), Some(false), "session 2's prompt was cancelled");
    assert_eq!(flag(4), None, "an unknown session's prompt was recorded");
    assert_eq!(flag(5), None, "session 0's prompt was recorded");
}

#[test]
fn two_prompts_with_the_same_jsonrpc_id_in_different_sessions_are_cancelled_one_at_a_time() {
    let (gw, _db) = shared("cancel-same-id");
    let backend = Arc::new(gw.session_backend());
    let owner = HeldOwner::hold(&gw);
    let (input_tx, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let (lines_tx, lines) = std::sync::mpsc::channel::<String>();
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
            super::acp::AcpServer::new(&session).serve(input, LineSink(lines_tx, Vec::new()))
        })
    };
    let send = |v: Value| input_tx.send(format!("{v}\n").into_bytes()).unwrap();
    send(req(1, "session/new", None));
    send(req(2, "session/new", None));
    let prompt = |s: &str| {
        json!({"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{
            "sessionId": s, "prompt": [{"type":"text","text":"who r"}]}})
    };
    // Session 2's prompt is the one running in the backend; session 1's, with the same JSON-RPC id, waits behind it.
    send(prompt("klatch-librarian-2"));
    wait_in_flight(&backend, 1);
    send(prompt("klatch-librarian-1"));
    send(
        json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":"klatch-librarian-1"}}),
    );
    // The cancel is for session 1's prompt, not the running one: the running look is not touched.
    let quiet = Instant::now() + Duration::from_millis(400);
    while let Ok(line) = lines.recv_timeout(quiet.saturating_duration_since(Instant::now())) {
        let m: Value = serde_json::from_str(&line).unwrap();
        assert!(
            m["id"] != 3,
            "a cancel for session 1 ended session 2's running prompt: {m}"
        );
    }
    assert_eq!(backend.in_flight(), 1);
    owner.release();
    let mut stops = Vec::new();
    while stops.len() < 2 {
        let line = lines
            .recv_timeout(WAIT)
            .unwrap_or_else(|_| panic!("a prompt never answered: {stops:?}"));
        let m: Value = serde_json::from_str(&line).unwrap();
        if m["id"] == 3 {
            stops.push(m["result"]["stopReason"].clone());
        }
    }
    assert_eq!(stops, vec![json!("end_turn"), json!("cancelled")]);
    drop(input_tx);
    served.join().unwrap().unwrap();
}
