//! The asks stop-waiting notes reach a member that only runs hooks (#789): derived from the room at each digest, so
//! they repeat while the condition holds and stop when it clears, with no table of what was reported.

use super::*;
use crate::gateway::gateway::lease::{LeaseSource, Observed};
use crate::harness::Verdict;
use std::cell::RefCell;

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

fn notes_of(g: &Gateway<SpyStore>, who: &Principal) -> Vec<String> {
    let d = g.digest(who).unwrap();
    d.rooms.iter().flat_map(|r| r.asks_notes.clone()).collect()
}

#[test]
fn an_asker_past_the_bound_hears_it_at_every_hook_until_the_ask_is_closed() {
    let (g, t) = timed();
    g.join(&alice(), "r").unwrap();
    g.post(&alice(), "r", "seeking: a thing").unwrap();
    assert!(notes_of(&g, &alice()).is_empty(), "too soon");
    t.set(t.get() + 400);
    let first = notes_of(&g, &alice());
    assert_eq!(first.len(), 1, "{first:?}");
    assert!(
        first[0].contains("your ask #1 has had no answer"),
        "{first:?}"
    );
    assert_eq!(notes_of(&g, &alice()), first, "repeats while it holds");
    assert_eq!(notes_of(&g, &alice()), first);
    g.post(&alice(), "r", "closed #1").unwrap();
    assert!(notes_of(&g, &alice()).is_empty(), "stops when it clears");
}

#[test]
fn the_members_of_an_ask_cycle_hear_it_and_it_clears_with_a_released_claim() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.join(&bob(), "r").unwrap();
    g.post(&bob(), "r", "seeking: b thing").unwrap();
    g.post(&alice(), "r", "seeking: a thing").unwrap();
    g.claim(&alice(), "r", &ClaimSpec::new("ask:1", 60, ""))
        .unwrap();
    g.claim(&bob(), "r", &ClaimSpec::new("ask:2", 60, ""))
        .unwrap();
    let a = notes_of(&g, &alice());
    let b = notes_of(&g, &bob());
    assert_eq!(a.len(), 1, "{a:?}");
    assert!(a[0].contains("ask cycle"), "{a:?}");
    assert!(b[0].contains("ask cycle"), "{b:?}");
    assert_eq!(notes_of(&g, &alice()), a, "repeats");
    g.release(&alice(), "r", "ask:1").unwrap();
    assert!(notes_of(&g, &alice()).is_empty());
    assert!(notes_of(&g, &bob()).is_empty());
}

#[test]
fn a_helper_hears_that_its_asker_process_is_gone_and_stops_when_it_is_not() {
    let fake = Fake::default();
    *fake.0.borrow_mut() = says(Verdict::Working);
    let g = open().with_leases(fake.clone());
    g.join(&alice(), "r").unwrap();
    g.join(&bob(), "r").unwrap();
    g.store_for_tests()
        .note_session("sess-a", "alice", 0)
        .unwrap();
    g.post(&alice(), "r", "seeking: a thing").unwrap();
    g.claim(&bob(), "r", &ClaimSpec::new("ask:1", 60, ""))
        .unwrap();
    assert!(notes_of(&g, &bob()).is_empty(), "alive");
    *fake.0.borrow_mut() = says(Verdict::Gone);
    let first = notes_of(&g, &bob());
    assert_eq!(first.len(), 1, "{first:?}");
    assert!(first[0].contains("alice has exited"), "{first:?}");
    assert_eq!(notes_of(&g, &bob()), first, "repeats while it holds");
    assert!(
        notes_of(&g, &alice()).is_empty(),
        "only the helper hears it"
    );
    *fake.0.borrow_mut() = says(Verdict::Working);
    assert!(notes_of(&g, &bob()).is_empty(), "stops when it clears");
}

#[test]
fn many_asks_keep_the_notes_bounded_and_the_digest_within_its_budget() {
    let (g, t) = timed();
    g.join(&alice(), "r").unwrap();
    for i in 0..40 {
        g.post(&alice(), "r", &format!("seeking: thing {i}"))
            .unwrap();
    }
    t.set(t.get() + 400);
    let d = g.digest(&alice()).unwrap();
    assert_eq!(
        d.rooms[0].asks_notes.len(),
        4,
        "{:?}",
        d.rooms[0].asks_notes
    );
    let out = crate::domain::render_hook(&d.rooms, d.now);
    assert!(
        crate::domain::printed_len(&out) <= crate::domain::budget::DIGEST_MAX_BYTES,
        "{} bytes",
        out.len()
    );
    assert!(out.contains("your ask #1 has had no answer"), "{out}");
}
