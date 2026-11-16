//! Replays traces of the Quint model (`models/claims.qnt`, DESIGN section 32 item 66, #405) against the real gateway
//! over an in-memory SQLite store, and compares what an observer can see after every step: the outcome of each claim
//! (taken, refused by whom, told to yield, over the cap), of each release, and the live claims with their owner,
//! scope and expiry.
//!
//! The traces are ITF JSON written by `quint run` or Apalache (`proto/scripts/quint-check traces`). A few are kept
//! under `models/traces` and replayed here, so a change to the claim, renewal, park or lease code that the model does
//! not allow fails an ordinary `cargo test` without Quint or Java. Point `KLATCH_MODEL_TRACES` at another directory
//! to replay traces you just made. One model minute is 60 real seconds.
//!
//! This is evidence that the code and the model agree on the traces replayed; it is not proof (RESEARCH-NOTES,
//! "Quint model of the claim and lease protocol").

use klatch::domain::{Error, Principal};
use klatch::gateway::{AllowAll, ClaimSpec, Clock, Gateway, LeaseSource, Observed};
use klatch::harness::Verdict;
use klatch::store::SqliteStore;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

const ROOM: &str = "r";
const MEMBERS: [&str; 3] = ["m1", "m2", "m3"];
/// A model minute is this many seconds; the model starts at `BASE`.
const MINUTE: i64 = 60;
const BASE: i64 = 1_000_000;

struct TestClock(Arc<AtomicI64>);
impl Clock for TestClock {
    fn now(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// The liveness probe the test controls: only the member named in the map is seen.
#[derive(Clone, Default)]
struct Probe(Rc<RefCell<HashMap<String, Verdict>>>);
impl LeaseSource for Probe {
    fn observe(&self, member: &str, _session: &str, _now_ms: u64) -> Option<Observed> {
        let verdict = *self.0.borrow().get(member)?;
        Some(Observed {
            verdict,
            quiet_for_ms: Some(5_000),
        })
    }
}

fn who(member: &str) -> Principal {
    Principal::unattributed(member)
}

/// A number in ITF: `{"#bigint": "12"}`.
fn int(v: &Value) -> i64 {
    v["#bigint"]
        .as_str()
        .unwrap_or_else(|| panic!("not an ITF integer: {v}"))
        .parse()
        .unwrap()
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or_else(|| panic!("no {key} in {v}"))
}

/// The live claims of a model state as `(path, owner, scope, expires in real seconds)`, sorted.
fn model_claims(state: &Value) -> Vec<(String, String, String, i64)> {
    let now = int(&state["now"]);
    let mut out: Vec<_> = state["claims"]["#set"]
        .as_array()
        .expect("claims is a set")
        .iter()
        .filter(|c| int(&c["expires"]) > now)
        .map(|c| {
            (
                text(c, "path").to_string(),
                text(c, "owner").to_string(),
                text(c, "scope").to_string(),
                BASE + MINUTE * int(&c["expires"]),
            )
        })
        .collect();
    out.sort();
    out
}

struct Replay {
    gw: Gateway<SqliteStore>,
    clock: Arc<AtomicI64>,
    probe: Probe,
    /// Makes each post differ from the last: the gateway refuses a repeat sent within moments.
    posts: std::cell::Cell<u32>,
}

impl Replay {
    fn new() -> Replay {
        let clock = Arc::new(AtomicI64::new(BASE));
        let probe = Probe::default();
        let gw = Gateway::new(
            SqliteStore::open_in_memory().unwrap(),
            TestClock(clock.clone()),
            AllowAll,
        )
        .with_leases(probe.clone());
        for m in MEMBERS {
            gw.join(&who(m), ROOM).unwrap();
            gw.note_session(&who(m), &format!("sess-{m}")).unwrap();
            // m1 and m2 share a scope, m3 has its own (scopeOf in the model)
            let scope = if m == "m3" { "s2" } else { "s1" };
            gw.set_scope(&who(m), scope).unwrap();
        }
        Replay {
            gw,
            clock,
            probe,
            posts: std::cell::Cell::new(0),
        }
    }

    fn claims(&self) -> Vec<(String, String, String, i64)> {
        let mut out: Vec<_> = self
            .gw
            .claims(&who("m1"), ROOM)
            .unwrap()
            .into_iter()
            .map(|c| (c.path, c.owner, c.scope, c.expires))
            .collect();
        out.sort();
        out
    }

    fn unique(&self, text: &str) -> String {
        self.posts.set(self.posts.get() + 1);
        format!("{text} {}", self.posts.get())
    }

    /// Take one step of the model and say what the real code did, in the model's words.
    fn apply(&self, step: &Value, state: &Value) -> String {
        let member = text(step, "member");
        let path = text(step, "path");
        match text(step, "kind") {
            "tick" => {
                self.clock
                    .store(BASE + MINUTE * int(&state["now"]), Ordering::SeqCst);
                String::new()
            }
            "claim" => self.claim(step),
            "release" => match self.gw.release(&who(member), ROOM, path) {
                Ok(()) => "ok".into(),
                Err(Error::NotFound(_)) => "notfound".into(),
                Err(e) => format!("error {e}"),
            },
            "renew" => {
                self.gw.read(&who(member), ROOM, false).unwrap();
                String::new()
            }
            "post" => {
                self.gw
                    .post(&who(member), ROOM, &self.unique("hello"))
                    .unwrap();
                String::new()
            }
            "park" => {
                let body = format!(
                    "parked: model {}, back by {}m",
                    self.unique("park"),
                    int(&step["span"])
                );
                self.gw.post(&who(member), ROOM, &body).unwrap();
                String::new()
            }
            "settle" => {
                let owner = text(step, "blocker");
                let verdict = match text(step, "verdict") {
                    "alive" => Verdict::Working,
                    "gone" => Verdict::Gone,
                    "stuck" => Verdict::Stuck,
                    _ => Verdict::Unknown,
                };
                self.probe.0.borrow_mut().insert(owner.into(), verdict);
                self.gw.claims(&who(member), ROOM).unwrap();
                self.probe.0.borrow_mut().clear();
                String::new()
            }
            other => panic!("unknown step kind {other}"),
        }
    }

    fn claim(&self, step: &Value) -> String {
        let spec = ClaimSpec::new(text(step, "path"), MINUTE * int(&step["ttl"]), "")
            .shared(step["shared"].as_bool().unwrap());
        match self.gw.claim(&who(text(step, "member")), ROOM, &spec) {
            Ok(()) => "ok".into(),
            Err(Error::Conflict {
                owner, deadlock, ..
            }) => format!("conflict by {owner} deadlock={deadlock}"),
            Err(Error::Invalid(_)) => "invalid".into(),
            Err(e) => format!("error {e}"),
        }
    }
}

/// What the model says the outcome of this step was, in the same words as `Replay::apply`.
fn model_outcome(step: &Value) -> String {
    match text(step, "kind") {
        "claim" => match text(step, "outcome") {
            "conflict" => format!(
                "conflict by {} deadlock={}",
                text(step, "blocker"),
                step["deadlock"].as_bool().unwrap()
            ),
            other => other.into(),
        },
        "release" => text(step, "outcome").into(),
        _ => String::new(),
    }
}

fn replay(file: &PathBuf) -> usize {
    let itf: Value = serde_json::from_str(&std::fs::read_to_string(file).unwrap())
        .unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    let states = itf["states"].as_array().expect("states");
    let real = Replay::new();
    assert_eq!(real.claims(), model_claims(&states[0]), "initial state");
    for (i, state) in states.iter().enumerate().skip(1) {
        let step = &state["last"];
        let done = real.apply(step, state);
        let at = format!("{} step {i}: {}", file.display(), step);
        assert_eq!(done, model_outcome(step), "outcome, {at}");
        assert_eq!(real.claims(), model_claims(state), "live claims, {at}");
    }
    states.len() - 1
}

fn trace_files() -> Vec<PathBuf> {
    let dir = std::env::var("KLATCH_MODEL_TRACES").map_or_else(
        |_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models/traces"),
        PathBuf::from,
    );
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().ends_with(".itf.json"))
        .collect();
    files.sort();
    files
}

#[test]
fn model_traces_replay_against_the_real_gateway() {
    let files = trace_files();
    assert!(!files.is_empty(), "no traces to replay");
    let steps: usize = files.iter().map(replay).sum();
    eprintln!("replayed {} traces, {steps} steps", files.len());
}
