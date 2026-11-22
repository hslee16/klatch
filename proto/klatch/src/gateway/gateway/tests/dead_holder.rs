//! A live claim whose holder's process is provably gone stands as a `dead_holder` stuck event (#1182). It only
//! flags: nothing is released, and unknown counts as alive, exactly as for `process_exited` (#790).

use super::*;
use crate::domain::StuckEvent;
use crate::gateway::gateway::lease::{LeaseSource, Observed};
use crate::harness::Verdict;
use std::cell::RefCell;

/// A made-up process table whose answer the test sets.
#[derive(Clone, Default)]
struct Fake(Rc<RefCell<Option<Observed>>>);

impl LeaseSource for Fake {
    fn observe(&self, _member: &str, _session: &str, _now_ms: u64) -> Option<Observed> {
        *self.0.borrow()
    }
}

fn says(verdict: Verdict) -> Option<Observed> {
    Some(Observed {
        verdict,
        quiet_for_ms: None,
    })
}

fn carol() -> Principal {
    Principal::unattributed("carol")
}

/// alice (with a session) holds `path`; carol watches. The table says Working while the claim is taken.
fn setup(path: &str) -> (Gateway<SpyStore>, Fake) {
    let fake = Fake::default();
    *fake.0.borrow_mut() = says(Verdict::Working);
    let g = open().with_leases(fake.clone());
    g.join(&alice(), "r").unwrap();
    g.join(&carol(), "r").unwrap();
    g.store_for_tests()
        .note_session("sess-a", "alice", 0)
        .unwrap();
    g.claim(&alice(), "r", &ClaimSpec::new(path, 500, ""))
        .unwrap();
    (g, fake)
}

fn dead(g: &Gateway<SpyStore>) -> Vec<StuckEvent> {
    g.stuck(&carol(), "r")
        .unwrap()
        .into_iter()
        .filter(|e| e.kind() == "dead_holder")
        .collect()
}

#[test]
fn a_claim_whose_holder_process_is_gone_is_a_dead_holder_event() {
    let (g, fake) = setup("src/a.rs");
    assert!(dead(&g).is_empty(), "alive");
    *fake.0.borrow_mut() = says(Verdict::Gone);
    assert_eq!(
        dead(&g),
        [StuckEvent::DeadHolder {
            room: "r".into(),
            claim: "src/a.rs".into(),
            holder: "alice".into()
        }]
    );
}

#[test]
fn an_ask_claim_is_named_by_its_ask_key() {
    let (g, fake) = setup("ask:7");
    *fake.0.borrow_mut() = says(Verdict::Gone);
    let ev = dead(&g);
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].key(), "dead_holder:r:ask:7");
}

#[test]
fn flagging_releases_nothing() {
    let (g, fake) = setup("src/a.rs");
    *fake.0.borrow_mut() = says(Verdict::Gone);
    assert_eq!(dead(&g).len(), 1);
    assert_eq!(dead(&g).len(), 1, "still flagged after a second read");
    let held = g.claims(&carol(), "r").unwrap();
    assert!(
        held.iter()
            .any(|c| c.owner == "alice" && c.path == "src/a.rs")
    );
}

#[test]
fn every_other_verdict_and_an_unreadable_record_flag_nothing() {
    let (g, fake) = setup("src/a.rs");
    for verdict in [
        Verdict::Working,
        Verdict::WaitingOnTool,
        Verdict::WaitingOnHuman,
        Verdict::Stuck,
        Verdict::Unknown,
    ] {
        *fake.0.borrow_mut() = says(verdict);
        assert!(dead(&g).is_empty(), "{verdict:?}");
    }
    *fake.0.borrow_mut() = None;
    assert!(dead(&g).is_empty());
}

#[test]
fn a_holder_with_no_session_record_is_never_probed() {
    let fake = Fake::default();
    *fake.0.borrow_mut() = says(Verdict::Gone);
    let g = open().with_leases(fake);
    g.join(&alice(), "r").unwrap();
    g.join(&carol(), "r").unwrap();
    g.claim(&alice(), "r", &ClaimSpec::new("p", 500, ""))
        .unwrap();
    assert!(dead(&g).is_empty());
}

#[test]
fn a_gateway_with_no_lease_source_flags_nothing() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.join(&carol(), "r").unwrap();
    g.store_for_tests()
        .note_session("sess-a", "alice", 0)
        .unwrap();
    g.claim(&alice(), "r", &ClaimSpec::new("p", 500, ""))
        .unwrap();
    assert!(dead(&g).is_empty());
}

#[test]
fn a_lapsed_claim_is_not_flagged() {
    let fake = Fake::default();
    *fake.0.borrow_mut() = says(Verdict::Working);
    let t = Rc::new(std::cell::Cell::new(1_000));
    let g = Gateway::new(SpyStore::new(), TestClock(t.clone()), AllowAll).with_leases(fake.clone());
    g.join(&alice(), "r").unwrap();
    g.join(&carol(), "r").unwrap();
    g.store_for_tests()
        .note_session("sess-a", "alice", 0)
        .unwrap();
    g.claim(&alice(), "r", &ClaimSpec::new("p", 10, ""))
        .unwrap();
    *fake.0.borrow_mut() = says(Verdict::Gone);
    t.set(t.get() + 11);
    assert!(dead(&g).is_empty());
}
