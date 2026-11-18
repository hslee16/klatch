//! The stuck-case events (#271): a guarded, read-only query that carries ids, names and numbers, never text.

use super::*;
use crate::domain::StuckEvent;

fn carol() -> Principal {
    Principal::unattributed("carol")
}

#[test]
fn an_old_unclaimed_ask_is_a_stale_ask_event_for_any_member_of_the_room() {
    let (g, t) = timed();
    g.join(&alice(), "r").unwrap();
    g.join(&bob(), "r").unwrap();
    let seq = g.post(&alice(), "r", "seeking: a thing").unwrap().seq;
    assert!(g.stuck(&bob(), "r").unwrap().is_empty(), "fresh");
    t.set(t.get() + 500);
    let ev = g.stuck(&bob(), "r").unwrap();
    assert_eq!(
        ev,
        [StuckEvent::StaleAsk {
            ask: seq,
            asker: "alice".into(),
            age_secs: 500
        }]
    );
}

#[test]
fn a_claim_deadlock_stands_as_an_event_not_only_as_a_refusal_flag() {
    let (g, _t) = timed();
    for p in [alice(), bob()] {
        g.join(&p, "r").unwrap();
    }
    g.claim(&alice(), "r", &ClaimSpec::new("p", 500, "")).unwrap();
    g.claim(&bob(), "r", &ClaimSpec::new("q", 500, "")).unwrap();
    assert!(g.claim(&alice(), "r", &ClaimSpec::new("q", 500, "")).is_err());
    assert!(g.claim(&bob(), "r", &ClaimSpec::new("p", 500, "")).is_err());
    let ev = g.stuck(&carol_in(&g), "r").unwrap();
    assert_eq!(
        ev,
        [StuckEvent::ClaimDeadlock {
            members: vec!["alice".into(), "bob".into()]
        }]
    );
}

fn carol_in(g: &Gateway<SpyStore>) -> Principal {
    g.join(&carol(), "r").unwrap();
    carol()
}

#[test]
fn a_cycle_through_asks_is_an_event_naming_the_yielder_first() {
    let (g, _t) = timed();
    for p in [alice(), bob()] {
        g.join(&p, "r").unwrap();
    }
    let a = g.post(&alice(), "r", "seeking: x").unwrap().seq;
    let b = g.post(&bob(), "r", "seeking: y").unwrap().seq;
    g.claim(&bob(), "r", &ClaimSpec::new(&format!("ask:{a}"), 90, "")).unwrap();
    g.claim(&alice(), "r", &ClaimSpec::new(&format!("ask:{b}"), 90, "")).unwrap();
    let ev = g.stuck(&alice(), "r").unwrap();
    assert_eq!(
        ev,
        [StuckEvent::AskCycle {
            members: vec!["alice".into(), "bob".into()],
            asks: vec![a, b]
        }]
    );
}

#[test]
fn a_member_below_the_rooms_label_cannot_read_the_events() {
    let g = open();
    g.admin_set_room_label(&admin(), "vault", "private").unwrap();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.join(&alice(), "vault").unwrap();
    g.post(&alice(), "vault", "seeking: x").unwrap();
    assert_eq!(rule_of(g.stuck(&bob(), "vault").unwrap_err()), "clearance");
}

#[test]
fn reading_the_events_writes_no_message_and_no_claim() {
    let (g, t) = timed();
    g.join(&alice(), "r").unwrap();
    g.post(&alice(), "r", "seeking: x").unwrap();
    t.set(t.get() + 500);
    let before = (g.tail(&alice(), "r", (0, 100)).unwrap().len(), g.claims(&alice(), "r").unwrap().len());
    g.stuck(&alice(), "r").unwrap();
    g.stuck(&alice(), "r").unwrap();
    let after = (g.tail(&alice(), "r", (0, 100)).unwrap().len(), g.claims(&alice(), "r").unwrap().len());
    assert_eq!(before, after);
}
