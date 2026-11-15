//! The lease rules driven by the liveness probe (#304): a claim past its TTL holds while its holder is alive, a
//! dead one lapses after the grace period, a stuck one at once, a hung busy one at the cap, and an unreadable one
//! keeps only its TTL.

use super::*;
use crate::gateway::gateway::lease::{
    LeaseAction, LeasePolicy, LeaseSource, Observed, lease_action,
};
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

fn seen(verdict: Verdict, quiet_s: Option<u64>) -> Option<Observed> {
    Some(Observed {
        verdict,
        quiet_for_ms: quiet_s.map(|s| s * 1000),
    })
}

/// alice holds `p` for 100 s from t = 1000 and has a session; bob will ask. Time is in the returned cell.
fn setup() -> (Gateway<SpyStore>, Rc<std::cell::Cell<i64>>, Fake) {
    let (g, t) = timed();
    let fake = Fake::default();
    let g = g.with_leases(fake.clone());
    g.store_for_tests()
        .note_session("sess-a", "alice", 0)
        .unwrap();
    g.claim(&alice(), "r", &ClaimSpec::new("p", 100, ""))
        .unwrap();
    (g, t, fake)
}

fn bob_can_take_p(g: &Gateway<SpyStore>) -> bool {
    g.claim(&bob(), "r", &ClaimSpec::new("p", 100, "")).is_ok()
}

#[test]
fn a_member_past_its_ttl_keeps_its_claim_while_working() {
    let (g, t, fake) = setup();
    t.set(1_000 + 500);
    *fake.0.borrow_mut() = seen(Verdict::Working, Some(5));
    assert!(!bob_can_take_p(&g), "alice is working, so p is still hers");
    assert_eq!(g.claims(&bob(), "r").unwrap()[0].owner, "alice");
}

#[test]
fn waiting_on_a_tool_and_waiting_on_a_human_hold_too() {
    for verdict in [Verdict::WaitingOnTool, Verdict::WaitingOnHuman] {
        let (g, t, fake) = setup();
        t.set(1_000 + 500);
        *fake.0.borrow_mut() = seen(verdict, Some(900));
        assert!(!bob_can_take_p(&g), "{verdict:?}");
    }
}

#[test]
fn a_dead_holder_lapses_after_the_grace_period_and_not_before() {
    let (g, t, fake) = setup();
    *fake.0.borrow_mut() = seen(Verdict::Gone, None);
    t.set(1_010);
    assert!(
        !bob_can_take_p(&g),
        "inside the grace period the claim lingers"
    );
    t.set(1_010 + 301);
    assert!(bob_can_take_p(&g), "the grace period is over");
}

#[test]
fn a_dead_holders_long_claim_is_cut_to_the_grace_period() {
    let (g, t, fake) = setup();
    g.claim(&alice(), "r", &ClaimSpec::new("long", 20_000, ""))
        .unwrap();
    *fake.0.borrow_mut() = seen(Verdict::Gone, None);
    t.set(1_010);
    g.claims(&bob(), "r").unwrap();
    t.set(1_010 + 301);
    assert!(
        g.claim(&bob(), "r", &ClaimSpec::new("long", 100, ""))
            .is_ok(),
        "a 20000 s claim does not outlive its dead holder by hours"
    );
}

#[test]
fn a_stuck_holders_claims_lapse_at_once() {
    let (g, t, fake) = setup();
    t.set(1_010);
    *fake.0.borrow_mut() = seen(Verdict::Stuck, Some(4000));
    assert!(bob_can_take_p(&g));
}

#[test]
fn a_hung_busy_holder_lapses_at_the_cap_and_a_raised_cap_holds_longer() {
    let (g, t, fake) = setup();
    t.set(1_500);
    *fake.0.borrow_mut() = seen(Verdict::WaitingOnTool, Some(4 * 3600 + 1));
    assert!(
        bob_can_take_p(&g),
        "no progress for longer than the default cap"
    );

    let (g, t, fake) = setup();
    g.store_for_tests()
        .policy_set("lease:cap", Some("86400"), 0)
        .unwrap();
    t.set(1_500);
    *fake.0.borrow_mut() = seen(Verdict::WaitingOnTool, Some(4 * 3600 + 1));
    assert!(!bob_can_take_p(&g), "a human raised the cap to a day");
}

#[test]
fn an_unreadable_holder_keeps_only_its_ttl() {
    let (g, t, fake) = setup();
    *fake.0.borrow_mut() = seen(Verdict::Unknown, None);
    t.set(1_050);
    assert!(
        !bob_can_take_p(&g),
        "unknown is not dead: inside the TTL it holds"
    );
    t.set(1_000 + 101);
    assert!(bob_can_take_p(&g), "and past the TTL it lapses as before");
}

#[test]
fn a_member_with_no_session_is_never_probed_and_keeps_its_ttl() {
    let (g, t) = timed();
    let fake = Fake::default();
    *fake.0.borrow_mut() = seen(Verdict::Stuck, None);
    let g = g.with_leases(fake);
    g.claim(&alice(), "r", &ClaimSpec::new("p", 100, ""))
        .unwrap();
    t.set(1_010);
    assert!(
        !bob_can_take_p(&g),
        "a bot's claim is not lapsed by a probe it has no process for"
    );
}

#[test]
fn without_a_lease_source_nothing_changes() {
    let (g, t) = timed();
    g.claim(&alice(), "r", &ClaimSpec::new("p", 100, ""))
        .unwrap();
    t.set(1_000 + 101);
    assert!(bob_can_take_p(&g));
}

#[test]
fn the_asker_is_not_probed_about_itself() {
    let (g, _t, fake) = setup();
    *fake.0.borrow_mut() = seen(Verdict::Stuck, Some(9999));
    assert_eq!(
        g.claims(&alice(), "r").unwrap().len(),
        1,
        "alice asking does not release her own claim"
    );
}

#[test]
fn the_rule_table() {
    let p = LeasePolicy {
        extend_secs: 600,
        grace_secs: 300,
        cap_secs: 3600,
    };
    let at = |v, q| {
        lease_action(
            Observed {
                verdict: v,
                quiet_for_ms: q,
            },
            1000,
            &p,
        )
    };
    assert_eq!(at(Verdict::Working, Some(1_000)), LeaseAction::Extend(1600));
    assert_eq!(at(Verdict::WaitingOnHuman, None), LeaseAction::Extend(1600));
    assert_eq!(
        at(Verdict::WaitingOnTool, Some(3_599_000)),
        LeaseAction::Extend(1600)
    );
    assert_eq!(
        at(Verdict::WaitingOnTool, Some(3_600_000)),
        LeaseAction::Release
    );
    assert_eq!(at(Verdict::Gone, None), LeaseAction::Shorten(1300));
    assert_eq!(at(Verdict::Stuck, None), LeaseAction::Release);
    assert_eq!(at(Verdict::Unknown, None), LeaseAction::Leave);
}
