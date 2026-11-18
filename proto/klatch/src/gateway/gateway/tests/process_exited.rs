//! Has the process behind another member's session exited? (#790, DESIGN item 96.) Only the probe's `Gone` says yes;
//! everything it cannot read, and every member it cannot probe, says no: unknown counts as alive.

use super::*;
use crate::gateway::gateway::lease::{LeaseSource, Observed};
use crate::harness::Verdict;
use std::cell::RefCell;

/// A made-up probe whose answer the test sets.
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

/// alice and bob share room `r`; alice has a session; the probe answers `fake`.
fn setup() -> (Gateway<SpyStore>, Fake) {
    let fake = Fake::default();
    let g = open().with_leases(fake.clone());
    g.join(&alice(), "r").unwrap();
    g.join(&bob(), "r").unwrap();
    g.store_for_tests()
        .note_session("sess-a", "alice", 0)
        .unwrap();
    (g, fake)
}

#[test]
fn a_gone_verdict_says_the_process_exited() {
    let (g, fake) = setup();
    *fake.0.borrow_mut() = says(Verdict::Gone);
    assert!(g.process_exited(&bob(), "r", "alice").unwrap());
}

#[test]
fn every_other_verdict_says_it_did_not() {
    let (g, fake) = setup();
    for verdict in [
        Verdict::Working,
        Verdict::WaitingOnTool,
        Verdict::WaitingOnHuman,
        Verdict::Stuck,
        Verdict::Unknown,
    ] {
        *fake.0.borrow_mut() = says(verdict);
        assert!(
            !g.process_exited(&bob(), "r", "alice").unwrap(),
            "{verdict:?}"
        );
    }
}

#[test]
fn a_record_the_probe_cannot_read_changes_nothing() {
    let (g, fake) = setup();
    *fake.0.borrow_mut() = None;
    assert!(!g.process_exited(&bob(), "r", "alice").unwrap());
}

#[test]
fn a_member_with_no_session_is_never_probed() {
    // bob has no session record (a bot): the fake would say Gone for anyone it was asked about
    let (g, fake) = setup();
    *fake.0.borrow_mut() = says(Verdict::Gone);
    assert!(!g.process_exited(&alice(), "r", "bob").unwrap());
}

#[test]
fn a_gateway_with_no_lease_source_says_no() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.join(&bob(), "r").unwrap();
    g.store_for_tests()
        .note_session("sess-a", "alice", 0)
        .unwrap();
    assert!(!g.process_exited(&bob(), "r", "alice").unwrap());
}

#[test]
fn only_a_member_of_the_room_is_answered_about() {
    // the room's access rule is the one for `who` (the guard decides who may look); within it, a name that is
    // not in the room is not probed, so a process is never reported for someone outside it
    let (g, fake) = setup();
    *fake.0.borrow_mut() = says(Verdict::Gone);
    let carol = Principal::unattributed("carol");
    g.join(&carol, "other").unwrap();
    assert!(!g.process_exited(&carol, "other", "alice").unwrap());
}
