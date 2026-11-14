use super::{Session, SessionConfig, serve};
use klatch_core::{Claim, Error, MemberInfo, Message, Posted, Principal, Result};
use klatch_gateway::{AllowAll, FixedClock, Gateway, Guard, Operation};
use klatch_protocol::{Attribution, ServerFrame, decode_server_line};
use klatch_store::{SqliteStore, Store};
use serde_json::{Value, json};
use std::io::Cursor;

fn gateway() -> Gateway<SqliteStore> {
    Gateway::new(
        SqliteStore::open_in_memory().unwrap(),
        FixedClock(1_000),
        AllowAll,
    )
}

/// Run a session over the given input lines and decode every output line through the real decoder.
fn run<S: Store>(g: &Gateway<S>, who: &Principal, lines: &[Value]) -> Vec<ServerFrame> {
    let input: String = lines.iter().map(|l| format!("{l}\n")).collect();
    let mut out = Vec::new();
    serve(g, who, Cursor::new(input), &mut out).unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| decode_server_line(l).unwrap_or_else(|e| panic!("bad output {l}: {e:?}")))
        .collect()
}

fn req(id: &str, method: &str, params: Value) -> Value {
    json!({"v":1,"type":"request","id":id,"method":method,"params":params})
}

fn alice() -> Principal {
    Principal::unattributed("alice")
}
fn bob() -> Principal {
    Principal::unattributed("bob")
}

fn response(frames: &[ServerFrame], id: &str) -> (bool, Option<Value>, Option<(String, String)>) {
    for f in frames {
        if let ServerFrame::Response {
            id: rid,
            ok,
            result,
            error,
            ..
        } = f
            && rid == id
        {
            return (
                *ok,
                result.clone(),
                error.as_ref().map(|e| (e.code.clone(), e.message.clone())),
            );
        }
    }
    panic!("no response for {id}: {frames:?}");
}

#[test]
fn the_first_frame_is_hello_with_the_transports_identity() {
    let g = gateway();
    let f = run(&g, &alice(), &[]);
    assert_eq!(f.len(), 1);
    assert!(
        matches!(&f[0], ServerFrame::Hello { member, attribution: Attribution::Unattributed, .. } if member == "alice")
    );
    let f = run(&g, &Principal::verified("carol"), &[]);
    assert!(matches!(
        &f[0],
        ServerFrame::Hello {
            attribution: Attribution::Verified,
            ..
        }
    ));
}

#[test]
fn every_request_gets_exactly_one_response_that_echoes_its_id_in_order() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[
            req("a", "post", json!({"room":"r","body":"x"})),
            req("b", "who", json!({"room":"r"})),
            req("c", "claims", json!({"room":"r"})),
        ],
    );
    let ids: Vec<&str> = f
        .iter()
        .filter_map(|x| {
            if let ServerFrame::Response { id, .. } = x {
                Some(id.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(ids, vec!["a", "b", "c"]);
}

#[test]
fn post_then_read_round_trips_and_the_sender_is_the_session_principal() {
    let g = gateway();
    run(
        &g,
        &alice(),
        &[req("p", "post", json!({"room":"r","body":"hello team"}))],
    );
    let f = run(
        &g,
        &bob(),
        &[
            req("j", "who", json!({"room":"r"})),
            req("r", "read", json!({"room":"r"})),
        ],
    );
    let (ok, result, _) = response(&f, "r");
    assert!(ok);
    let m = &result.unwrap()["messages"][0];
    assert_eq!(
        (m["sender"].as_str(), m["body"].as_str(), m["seq"].as_u64()),
        (Some("alice"), Some("hello team"), Some(1))
    );
    assert_eq!(m["attribution"], "unattributed");
    assert!(m["id"].as_str().unwrap().len() == 26);
}

#[test]
fn a_request_cannot_choose_who_it_acts_as() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[req(
            "p",
            "post",
            json!({"room":"r","body":"x","member":"bob","sender":"bob"}),
        )],
    );
    assert!(response(&f, "p").0);
    let f = run(&g, &bob(), &[req("r", "read", json!({"room":"r"}))]);
    let (_, result, _) = response(&f, "r");
    assert_eq!(result.unwrap()["messages"][0]["sender"], "alice");
}

#[test]
fn read_advances_the_cursor_and_peek_does_not() {
    let g = gateway();
    run(
        &g,
        &alice(),
        &[req("p", "post", json!({"room":"r","body":"x"}))],
    );
    let f = run(
        &g,
        &bob(),
        &[
            req("1", "read", json!({"room":"r","peek":true})),
            req("2", "read", json!({"room":"r","peek":true})),
            req("3", "read", json!({"room":"r"})),
            req("4", "read", json!({"room":"r"})),
        ],
    );
    let n = |id: &str| {
        response(&f, id).1.unwrap()["messages"]
            .as_array()
            .unwrap()
            .len()
    };
    assert_eq!((n("1"), n("2"), n("3"), n("4")), (1, 1, 1, 0));
}

#[test]
fn who_lists_members_with_last_seen() {
    let g = gateway();
    run(
        &g,
        &alice(),
        &[req("j", "post", json!({"room":"r","body":"x"}))],
    );
    let f = run(&g, &bob(), &[req("w", "who", json!({"room":"r"}))]);
    let members = response(&f, "w").1.unwrap()["members"].clone();
    assert_eq!(members[0]["name"], "alice");
    assert_eq!(members[0]["last_seen"], 1000);
}

#[test]
fn claims_conflict_release_and_list_over_the_wire() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[req(
            "c",
            "claim",
            json!({"room":"r","path":"src","reason":"refactor"}),
        )],
    );
    assert!(response(&f, "c").0);
    let f = run(
        &g,
        &bob(),
        &[
            req("c", "claim", json!({"room":"r","path":"src/a.rs"})),
            req("l", "claims", json!({"room":"r"})),
        ],
    );
    let (ok, _, err) = response(&f, "c");
    assert!(!ok);
    let (code, msg) = err.unwrap();
    assert_eq!(code, "conflict");
    assert!(msg.contains("src is claimed by alice"), "{msg}");
    let claims = response(&f, "l").1.unwrap()["claims"].clone();
    assert_eq!(
        (
            claims[0]["path"].as_str(),
            claims[0]["owner"].as_str(),
            claims[0]["reason"].as_str()
        ),
        (Some("src"), Some("alice"), Some("refactor"))
    );
    let f = run(
        &g,
        &alice(),
        &[req("r", "release", json!({"room":"r","path":"src"}))],
    );
    assert!(response(&f, "r").0);
    let f = run(
        &g,
        &bob(),
        &[req("c", "claim", json!({"room":"r","path":"src/a.rs"}))],
    );
    assert!(response(&f, "c").0);
}

#[test]
fn a_claim_defaults_its_ttl_and_honours_an_explicit_one() {
    let g = gateway();
    run(
        &g,
        &alice(),
        &[
            req("a", "claim", json!({"room":"r","path":"one"})),
            req("b", "claim", json!({"room":"r","path":"two","ttl":30})),
        ],
    );
    let f = run(&g, &bob(), &[req("l", "claims", json!({"room":"r"}))]);
    let c = response(&f, "l").1.unwrap()["claims"].clone();
    assert_eq!(c[0]["expires"], 1000 + 600);
    assert_eq!(c[1]["expires"], 1000 + 30);
}

#[test]
fn releasing_a_claim_you_do_not_hold_is_not_found() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[req("r", "release", json!({"room":"r","path":"src"}))],
    );
    assert_eq!(response(&f, "r").2.unwrap().0, "not_found");
}

#[test]
fn invalid_input_and_unknown_methods_get_distinct_error_codes() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[
            req("1", "post", json!({"body":"no room"})),
            req("2", "post", json!({"room":"bad room","body":"x"})),
            req("3", "teleport", json!({})),
            json!({"v":1,"type":"request","id":"4","method":"who"}),
        ],
    );
    assert_eq!(response(&f, "1").2.unwrap().0, "invalid_params");
    assert_eq!(response(&f, "2").2.unwrap().0, "invalid");
    assert_eq!(response(&f, "3").2.unwrap().0, "unknown_method");
    assert_eq!(response(&f, "4").2.unwrap().0, "invalid_params");
}

struct DenyRoom(&'static str);
impl Guard for DenyRoom {
    fn check(&self, _who: &Principal, op: &Operation) -> Result<()> {
        if let Operation::Post { room, .. } = op
            && room == self.0
        {
            return Err(Error::Refused {
                rule: "test.deny".into(),
                reason: "that room is closed".into(),
            });
        }
        Ok(())
    }
}

#[test]
fn a_policy_refusal_sends_a_refusal_frame_then_a_failed_response_and_never_echoes_the_body() {
    let g = Gateway::new(
        SqliteStore::open_in_memory().unwrap(),
        FixedClock(1),
        DenyRoom("locked"),
    );
    let f = run(
        &g,
        &alice(),
        &[req(
            "p",
            "post",
            json!({"room":"locked","body":"SECRET-BODY-TEXT"}),
        )],
    );
    assert!(
        matches!(&f[1], ServerFrame::Refusal { request: Some(r), rule, .. } if r == "p" && rule == "test.deny")
    );
    let (ok, _, err) = response(&f, "p");
    assert!(!ok);
    assert_eq!(err.unwrap().0, "refused");
    assert!(!format!("{f:?}").contains("SECRET-BODY-TEXT"));
}

#[test]
fn garbage_and_wrong_version_lines_are_refused_and_the_session_carries_on() {
    let g = gateway();
    let input = "this is not json\n{\"v\":2,\"type\":\"request\",\"id\":\"x\",\"method\":\"who\"}\n"
        .to_string() + &format!("{}\n", req("ok", "who", json!({"room":"r"})));
    let mut out = Vec::new();
    serve(&g, &alice(), Cursor::new(input), &mut out).unwrap();
    let f: Vec<ServerFrame> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| decode_server_line(l).unwrap())
        .collect();
    let rules: Vec<&str> = f
        .iter()
        .filter_map(|x| {
            if let ServerFrame::Refusal { rule, .. } = x {
                Some(rule.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(rules, vec!["protocol.decode", "protocol.version"]);
    assert!(response(&f, "ok").0);
}

#[test]
fn an_oversize_line_is_refused_and_the_session_carries_on() {
    let g = gateway();
    let huge = "x".repeat(klatch_protocol::MAX_LINE_BYTES * 2);
    let input = format!("{huge}\n{}\n", req("ok", "who", json!({"room":"r"})));
    let mut out = Vec::new();
    serve(&g, &alice(), Cursor::new(input), &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("protocol.decode"));
    assert!(text.contains("\"id\":\"ok\""));
}

#[test]
fn acks_and_unknown_frames_are_accepted_silently() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[
            json!({"v":1,"type":"ack","room":"r","seq":3}),
            json!({"v":1,"type":"something_new","x":1}),
        ],
    );
    assert_eq!(f.len(), 1, "only hello: {f:?}");
}

struct DownStore;
impl Store for DownStore {
    fn join(&self, _: &str, _: &str, _: i64) -> Result<()> {
        Err(Error::Backend("down".into()))
    }
    fn post_as(
        &self,
        _: &str,
        _: &str,
        _: klatch_core::Attribution,
        _: &str,
        _: &str,
        _: i64,
    ) -> Result<Posted> {
        Err(Error::Backend("down".into()))
    }
    fn read(&self, _: &str, _: &str, _: bool, _: i64) -> Result<Vec<Message>> {
        Err(Error::Busy("locked".into()))
    }
    fn members(&self, _: &str) -> Result<Vec<MemberInfo>> {
        Err(Error::Backend("down".into()))
    }
    fn claim(&self, _: &str, _: &str, _: &str, _: i64, _: &str, _: i64) -> Result<()> {
        Err(Error::Backend("down".into()))
    }
    fn release(&self, _: &str, _: &str, _: &str) -> Result<()> {
        Err(Error::Backend("down".into()))
    }
    fn claims(&self, _: &str, _: i64) -> Result<Vec<Claim>> {
        Err(Error::Backend("down".into()))
    }
    fn rooms_of(&self, _: &str) -> Result<Vec<String>> {
        Err(Error::Backend("down".into()))
    }
    fn tail(&self, _: &str, _: i64, _: usize) -> Result<Vec<Message>> {
        Err(Error::Backend("down".into()))
    }
    fn head(&self, _: &str) -> Result<i64> {
        Err(Error::Backend("down".into()))
    }
    fn advance_cursor(&self, _: &str, _: &str, _: i64, _: i64) -> Result<()> {
        Err(Error::Backend("down".into()))
    }
}

#[test]
fn store_failures_become_failed_responses_and_the_session_survives() {
    let g = Gateway::new(DownStore, FixedClock(1), AllowAll);
    let f = run(
        &g,
        &alice(),
        &[
            req("1", "post", json!({"room":"r","body":"x"})),
            req("2", "read", json!({"room":"r"})),
            req("3", "who", json!({"room":"r"})),
        ],
    );
    assert_eq!(response(&f, "1").2.unwrap().0, "backend");
    assert_eq!(response(&f, "2").2.unwrap().0, "busy");
    assert_eq!(response(&f, "3").2.unwrap().0, "backend");
}

#[test]
fn the_session_ends_cleanly_at_end_of_input() {
    let g = gateway();
    let mut out = Vec::new();
    assert!(serve(&g, &alice(), Cursor::new(""), &mut out).is_ok());
}

#[test]
fn every_output_line_is_one_valid_frame() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[
            req("p", "post", json!({"room":"r","body":"line one\nline two"})),
            req("r", "read", json!({"room":"r"})),
        ],
    );
    assert!(f.len() >= 3); // decoding every line already proved validity
}

// ---- streaming: subscribe, replay, live push, heartbeat, ack, resume (#95) ----

use klatch_store::SqliteStore as Sqlite;
use std::io::{BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn frames_of(out: &[u8]) -> Vec<ServerFrame> {
    String::from_utf8(out.to_vec())
        .unwrap()
        .lines()
        .map(|l| decode_server_line(l).unwrap_or_else(|e| panic!("bad output {l}: {e:?}")))
        .collect()
}

fn messages(f: &[ServerFrame]) -> Vec<(String, u64, String)> {
    f.iter()
        .filter_map(|x| match x {
            ServerFrame::Message {
                room, seq, body, ..
            } => Some((room.clone(), *seq, body.clone())),
            _ => None,
        })
        .collect()
}

fn seqs(f: &[ServerFrame]) -> Vec<u64> {
    messages(f).into_iter().map(|m| m.1).collect()
}

#[test]
fn subscribe_replays_history_then_caught_up_when_the_input_ends() {
    let g = gateway();
    for i in 0..3 {
        g.post(&bob(), "r", &format!("m{i}")).unwrap();
    }
    let f = run(
        &g,
        &alice(),
        &[req("s", "subscribe", json!({"room":"r","from_seq":0}))],
    );
    let (ok, result, _) = response(&f, "s");
    assert!(ok);
    assert_eq!(result.unwrap()["head"], 3);
    assert_eq!(seqs(&f), vec![1, 2, 3]);
    let tail = f.last().unwrap();
    assert!(
        matches!(tail, ServerFrame::CaughtUp { room, seq: 3, .. } if room == "r"),
        "{tail:?}"
    );
    let resp_at = f
        .iter()
        .position(|x| matches!(x, ServerFrame::Response { .. }))
        .unwrap();
    let first_msg = f
        .iter()
        .position(|x| matches!(x, ServerFrame::Message { .. }))
        .unwrap();
    assert!(resp_at < first_msg, "the response comes before the replay");
}

#[test]
fn subscribe_from_a_seq_skips_what_the_client_already_has() {
    let g = gateway();
    for i in 0..4 {
        g.post(&bob(), "r", &format!("m{i}")).unwrap();
    }
    let f = run(
        &g,
        &alice(),
        &[req("s", "subscribe", json!({"room":"r","from_seq":2}))],
    );
    assert_eq!(seqs(&f), vec![3, 4]);
}

#[test]
fn subscribing_to_an_empty_room_reports_caught_up_at_zero() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[req("s", "subscribe", json!({"room":"r","from_seq":0}))],
    );
    assert!(matches!(
        f.last().unwrap(),
        ServerFrame::CaughtUp { seq: 0, .. }
    ));
}

#[test]
fn the_stream_includes_the_subscribers_own_messages() {
    let g = gateway();
    g.post(&alice(), "r", "mine").unwrap();
    g.post(&bob(), "r", "theirs").unwrap();
    let f = run(
        &g,
        &alice(),
        &[req("s", "subscribe", json!({"room":"r","from_seq":0}))],
    );
    assert_eq!(seqs(&f), vec![1, 2]);
}

#[test]
fn a_bad_room_or_missing_from_seq_is_refused_at_subscribe_time() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[
            req("1", "subscribe", json!({"room":"bad room","from_seq":0})),
            req("2", "subscribe", json!({"room":"r"})),
            req("3", "subscribe", json!({"room":"r","from_seq":-4})),
        ],
    );
    assert_eq!(response(&f, "1").2.unwrap().0, "invalid");
    assert_eq!(response(&f, "2").2.unwrap().0, "invalid_params");
    assert_eq!(response(&f, "3").2.unwrap().0, "invalid_params");
    assert!(messages(&f).is_empty());
}

struct DenyStream;
impl Guard for DenyStream {
    fn check(&self, _who: &Principal, op: &Operation) -> Result<()> {
        if matches!(op, Operation::Subscribe { .. }) {
            return Err(Error::Refused {
                rule: "test.no_stream".into(),
                reason: "streaming is not allowed here".into(),
            });
        }
        Ok(())
    }
}

#[test]
fn a_guard_can_refuse_a_subscription_and_nothing_streams() {
    let g = Gateway::new(
        SqliteStore::open_in_memory().unwrap(),
        FixedClock(1),
        DenyStream,
    );
    g.post(&bob(), "r", "secret").unwrap();
    let f = run(
        &g,
        &alice(),
        &[req("s", "subscribe", json!({"room":"r","from_seq":0}))],
    );
    assert!(f.iter().any(|x| matches!(x, ServerFrame::Refusal { request: Some(r), rule, .. } if r == "s" && rule == "test.no_stream")));
    assert_eq!(response(&f, "s").2.unwrap().0, "refused");
    assert!(messages(&f).is_empty());
}

#[test]
fn a_store_failure_at_subscribe_is_a_failed_response() {
    let g = Gateway::new(DownStore, FixedClock(1), AllowAll);
    let f = run(
        &g,
        &alice(),
        &[req("s", "subscribe", json!({"room":"r","from_seq":0}))],
    );
    assert_eq!(response(&f, "s").2.unwrap().0, "backend");
}

#[test]
fn unsubscribing_is_idempotent_and_answers_ok() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[req("u", "unsubscribe", json!({"room":"r"}))],
    );
    assert!(response(&f, "u").0);
}

#[test]
fn an_ack_frame_advances_the_members_read_cursor() {
    let g = gateway();
    g.join(&alice(), "r").unwrap();
    for i in 0..3 {
        g.post(&bob(), "r", &format!("m{i}")).unwrap();
    }
    run(
        &g,
        &alice(),
        &[json!({"v":1,"type":"ack","room":"r","seq":2})],
    );
    let left = g.read(&alice(), "r", false).unwrap();
    assert_eq!(left.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![3]);
}

// ---- live sessions: a thread, a channel-backed input, a shared output, a file database ----

struct ChanReader {
    rx: Receiver<Vec<u8>>,
    pending: Vec<u8>,
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
struct SharedOut(Arc<Mutex<Vec<u8>>>);
impl Write for SharedOut {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn fast() -> SessionConfig {
    SessionConfig {
        poll: Duration::from_millis(5),
        heartbeat: Duration::from_secs(60),
        window: 100,
        batch: 64,
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
        Sqlite::open(&path).unwrap();
        let (tx, rx) = channel();
        let out = SharedOut(Arc::new(Mutex::new(Vec::new())));
        let (o2, p2) = (out.clone(), path.clone());
        let handle = std::thread::spawn(move || {
            let store = Sqlite::open(&p2).unwrap();
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
            Sqlite::open(&self.path).unwrap(),
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
impl Store for FlakyStore {
    fn join(&self, r: &str, m: &str, n: i64) -> Result<()> {
        self.inner.join(r, m, n)
    }
    fn post_as(
        &self,
        r: &str,
        s: &str,
        a: klatch_core::Attribution,
        l: &str,
        b: &str,
        n: i64,
    ) -> Result<Posted> {
        self.inner.post_as(r, s, a, l, b, n)
    }
    fn read(&self, r: &str, m: &str, a: bool, n: i64) -> Result<Vec<Message>> {
        self.inner.read(r, m, a, n)
    }
    fn members(&self, r: &str) -> Result<Vec<MemberInfo>> {
        self.inner.members(r)
    }
    fn claim(&self, r: &str, o: &str, p: &str, t: i64, why: &str, n: i64) -> Result<()> {
        self.inner.claim(r, o, p, t, why, n)
    }
    fn release(&self, r: &str, o: &str, p: &str) -> Result<()> {
        self.inner.release(r, o, p)
    }
    fn claims(&self, r: &str, n: i64) -> Result<Vec<Claim>> {
        self.inner.claims(r, n)
    }
    fn rooms_of(&self, m: &str) -> Result<Vec<String>> {
        self.inner.rooms_of(m)
    }
    fn tail(&self, r: &str, a: i64, l: usize) -> Result<Vec<Message>> {
        self.gate()?;
        self.inner.tail(r, a, l)
    }
    fn head(&self, r: &str) -> Result<i64> {
        self.inner.head(r)
    }
    fn advance_cursor(&self, r: &str, m: &str, s: i64, n: i64) -> Result<()> {
        self.inner.advance_cursor(r, m, s, n)
    }
}

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

// ---- stored attribution and label are reported, not placeholders (#139) ----

#[test]
fn read_and_the_stream_report_the_stored_attribution_and_label() {
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .post_as(
            "r",
            "carol",
            klatch_core::Attribution::Verified,
            "internal",
            "proven",
            1,
        )
        .unwrap();
    store
        .post_as(
            "r",
            "eve",
            klatch_core::Attribution::Unattributed,
            "public",
            "unproven",
            2,
        )
        .unwrap();
    let g = Gateway::new(store, FixedClock(5), AllowAll);
    let f = run(
        &g,
        &bob(),
        &[
            req("r", "read", json!({"room":"r","peek":true})),
            req("s", "subscribe", json!({"room":"r","from_seq":0})),
        ],
    );
    let read = response(&f, "r").1.unwrap();
    let m = read["messages"].as_array().unwrap();
    assert_eq!(
        (m[0]["attribution"].as_str(), m[0]["label"].as_str()),
        (Some("verified"), Some("internal"))
    );
    assert_eq!(
        (m[1]["attribution"].as_str(), m[1]["label"].as_str()),
        (Some("unattributed"), Some("public"))
    );
    let streamed: Vec<_> = f
        .iter()
        .filter_map(|x| match x {
            ServerFrame::Message {
                attribution, label, ..
            } => Some((attribution.clone(), label.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        streamed,
        vec![
            (Attribution::Verified, "internal".to_string()),
            (Attribution::Unattributed, "public".to_string())
        ]
    );
}

#[test]
fn a_message_posted_through_a_verified_session_reads_back_verified() {
    let g = gateway();
    run(
        &g,
        &Principal::verified("carol"),
        &[req("p", "post", json!({"room":"r","body":"hello"}))],
    );
    let f = run(&g, &bob(), &[req("r", "read", json!({"room":"r"}))]);
    assert_eq!(
        response(&f, "r").1.unwrap()["messages"][0]["attribution"],
        "verified"
    );
}

#[test]
fn an_empty_post_body_is_an_invalid_request_over_the_wire() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[req("p", "post", json!({"room":"r","body":"   "}))],
    );
    assert_eq!(response(&f, "p").2.unwrap().0, "invalid");
}

#[test]
fn status_is_set_by_the_caller_and_shown_by_who() {
    let g = gateway();
    run(
        &g,
        &alice(),
        &[req("j", "post", json!({"room":"r","body":"hi"}))],
    );
    let f = run(
        &g,
        &alice(),
        &[
            req("s", "status", json!({"room":"r","status":"done leaving"})),
            req("w", "who", json!({"room":"r"})),
        ],
    );
    assert!(response(&f, "s").0, "status failed");
    let members = response(&f, "w").1.unwrap()["members"].clone();
    let a = members
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "alice")
        .unwrap();
    assert_eq!(a["status"], "done leaving");
}

#[test]
fn a_bad_status_is_an_invalid_error() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[req("s", "status", json!({"room":"r","status":""}))],
    );
    let (ok, _, err) = response(&f, "s");
    assert!(!ok);
    assert_eq!(err.unwrap().0, "invalid");
}

#[test]
fn scopes_let_two_members_claim_the_same_path_and_a_shared_claim_reaches_every_scope() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[
            req("s", "scope", json!({"scope":"git:a"})),
            req("c", "claim", json!({"room":"r","path":"src/lib.rs"})),
        ],
    );
    assert!(response(&f, "s").0 && response(&f, "c").0);
    let f = run(
        &g,
        &bob(),
        &[
            req("s", "scope", json!({"scope":"git:b"})),
            req("c", "claim", json!({"room":"r","path":"src/lib.rs"})),
            req(
                "d",
                "claim",
                json!({"room":"r","path":"docs","shared":true}),
            ),
            req("l", "claims", json!({"room":"r"})),
        ],
    );
    assert!(
        response(&f, "c").0,
        "a different scope may claim the same path"
    );
    assert!(response(&f, "d").0);
    let claims = response(&f, "l").1.unwrap()["claims"].clone();
    let scopes: Vec<_> = claims
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["scope"].as_str().unwrap_or("").to_string())
        .collect();
    assert!(
        scopes.contains(&"git:a".to_string())
            && scopes.contains(&"git:b".to_string())
            && scopes.contains(&"*".to_string()),
        "{scopes:?}"
    );
    let f = run(
        &g,
        &alice(),
        &[req("x", "claim", json!({"room":"r","path":"docs/api.md"}))],
    );
    let (ok, _, err) = response(&f, "x");
    assert!(!ok);
    assert_eq!(err.unwrap().0, "conflict");
}

#[test]
fn a_bad_scope_is_an_invalid_error_over_the_wire() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[req("s", "scope", json!({"scope":"has space"}))],
    );
    let (ok, _, err) = response(&f, "s");
    assert!(!ok);
    assert_eq!(err.unwrap().0, "invalid");
}
