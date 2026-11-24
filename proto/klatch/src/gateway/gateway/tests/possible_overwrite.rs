//! A claim taken on a path that was just released after a deadlock yield stands as a `possible_overwrite` stuck event
//! (#909, option C). It flags the pattern and never judges whether the second edit built on the first.

use super::*;
use crate::domain::StuckEvent;

fn carol() -> Principal {
    Principal::unattributed("carol")
}

fn claim(g: &Gateway<SpyStore>, who: &Principal, path: &str) -> Result<()> {
    g.claim(who, "r", &ClaimSpec::new(path, 5_000, ""))
}

/// alice holds p, bob holds q, alice waits for q, bob's attempt on p closes the cycle (a deadlock), and carol watches.
fn deadlocked() -> (Gateway<SpyStore>, Rc<std::cell::Cell<i64>>) {
    let (g, t) = timed();
    for p in [alice(), bob(), carol()] {
        g.join(&p, "r").unwrap();
    }
    claim(&g, &alice(), "p").unwrap();
    claim(&g, &bob(), "q").unwrap();
    assert!(claim(&g, &alice(), "q").is_err());
    assert!(claim(&g, &bob(), "p").is_err());
    (g, t)
}

fn overwrites(g: &Gateway<SpyStore>) -> Vec<StuckEvent> {
    g.stuck(&carol(), "r")
        .unwrap()
        .into_iter()
        .filter(|e| e.kind() == "possible_overwrite")
        .collect()
}

fn event(path: &str, previous: &str, new: &str) -> StuckEvent {
    StuckEvent::PossibleOverwrite {
        room: "r".into(),
        path: path.into(),
        previous: previous.into(),
        new: new.into(),
    }
}

#[test]
fn a_reclaim_of_a_released_path_after_a_yield_by_another_member_is_one_event() {
    let (g, _t) = deadlocked();
    assert!(overwrites(&g).is_empty(), "nothing released yet");
    g.release(&alice(), "r", "p").unwrap();
    assert!(overwrites(&g).is_empty(), "nobody took it yet");
    claim(&g, &bob(), "p").unwrap();
    assert_eq!(overwrites(&g), [event("p", "alice", "bob")]);
    assert_eq!(overwrites(&g).len(), 1, "read again: still one, not two");
    assert_eq!(overwrites(&g)[0].key(), "possible_overwrite:r:p:alice:bob");
}

#[test]
fn a_release_with_a_note_counts_the_same() {
    let (g, _t) = deadlocked();
    g.release_with_note(&alice(), ("r", "p"), "changed it")
        .unwrap();
    claim(&g, &bob(), "p").unwrap();
    assert_eq!(overwrites(&g), [event("p", "alice", "bob")]);
}

#[test]
fn no_event_without_a_prior_yield() {
    let (g, _t) = timed();
    for p in [alice(), bob(), carol()] {
        g.join(&p, "r").unwrap();
    }
    claim(&g, &alice(), "p").unwrap();
    g.release(&alice(), "r", "p").unwrap();
    claim(&g, &bob(), "p").unwrap();
    assert!(overwrites(&g).is_empty(), "a plain hand over");
}

#[test]
fn a_plain_conflict_that_is_not_a_deadlock_is_no_yield() {
    let (g, _t) = timed();
    for p in [alice(), bob(), carol()] {
        g.join(&p, "r").unwrap();
    }
    claim(&g, &alice(), "p").unwrap();
    assert!(claim(&g, &bob(), "p").is_err(), "a conflict, not a cycle");
    g.release(&alice(), "r", "p").unwrap();
    claim(&g, &bob(), "p").unwrap();
    assert!(overwrites(&g).is_empty());
}

#[test]
fn the_same_member_taking_its_own_path_again_is_not_an_overwrite() {
    let (g, _t) = deadlocked();
    g.release(&alice(), "r", "p").unwrap();
    claim(&g, &alice(), "p").unwrap();
    assert!(overwrites(&g).is_empty());
}

#[test]
fn a_member_outside_the_deadlock_releasing_is_not_a_yield() {
    let (g, _t) = deadlocked();
    claim(&g, &carol(), "z").unwrap();
    g.release(&carol(), "r", "z").unwrap();
    claim(&g, &bob(), "z").unwrap();
    assert!(overwrites(&g).is_empty());
}

#[test]
fn a_take_after_the_window_is_not_flagged_and_a_flag_ends_with_the_window() {
    let (g, t) = deadlocked();
    g.release(&alice(), "r", "p").unwrap();
    t.set(t.get() + crate::domain::YIELD_WINDOW_SECS + 1);
    claim(&g, &bob(), "p").unwrap();
    assert!(overwrites(&g).is_empty(), "taken after the window");

    let (g, t) = deadlocked();
    g.release(&alice(), "r", "p").unwrap();
    claim(&g, &bob(), "p").unwrap();
    assert_eq!(overwrites(&g).len(), 1);
    t.set(t.get() + crate::domain::YIELD_WINDOW_SECS + 1);
    assert!(overwrites(&g).is_empty(), "the flag ends with the window");
}

#[test]
fn a_path_that_is_not_plain_is_shown_as_unnamed() {
    let (g, _t) = timed();
    for p in [alice(), bob(), carol()] {
        g.join(&p, "r").unwrap();
    }
    let odd = "dir/a file.rs";
    claim(&g, &alice(), odd).unwrap();
    claim(&g, &bob(), "q").unwrap();
    assert!(claim(&g, &alice(), "q").is_err());
    assert!(claim(&g, &bob(), odd).is_err());
    g.release(&alice(), "r", odd).unwrap();
    claim(&g, &bob(), odd).unwrap();
    assert_eq!(overwrites(&g), [event("(unnamed)", "alice", "bob")]);
}

/// As `deadlocked`, but each claim lives `ttl` seconds and may be anchored to `anchor` (#1224).
fn deadlocked_with(
    ttl: i64,
    anchor: &'static str,
) -> (Gateway<SpyStore>, Rc<std::cell::Cell<i64>>) {
    let (g, t) = timed();
    for p in [alice(), bob(), carol()] {
        g.join(&p, "r").unwrap();
    }
    let spec = |path| {
        let s = ClaimSpec::new(path, ttl, "");
        if anchor.is_empty() {
            s
        } else {
            s.anchored(anchor)
        }
    };
    g.claim(&alice(), "r", &spec("p")).unwrap();
    g.claim(&bob(), "r", &spec("q")).unwrap();
    assert!(g.claim(&alice(), "r", &spec("q")).is_err());
    assert!(g.claim(&bob(), "r", &spec("p")).is_err());
    (g, t)
}

#[test]
fn a_yield_that_ends_by_the_lease_lapsing_is_flagged_when_another_takes_the_path() {
    let (g, t) = deadlocked_with(100, "");
    t.set(t.get() + 101);
    claim(&g, &bob(), "p").unwrap();
    assert_eq!(overwrites(&g), [event("p", "alice", "bob")]);
}

#[test]
fn a_lapse_long_before_the_take_is_outside_the_window() {
    let (g, t) = deadlocked_with(100, "");
    t.set(t.get() + 101 + crate::domain::YIELD_WINDOW_SECS);
    claim(&g, &bob(), "p").unwrap();
    assert!(overwrites(&g).is_empty());
}

#[test]
fn a_yield_that_ends_by_the_session_ending_is_flagged() {
    let (g, _t) = deadlocked();
    assert_eq!(g.release_all(&alice()).unwrap(), 1);
    claim(&g, &bob(), "p").unwrap();
    assert_eq!(overwrites(&g), [event("p", "alice", "bob")]);
}

#[test]
fn a_yield_that_ends_by_an_anchor_release_is_flagged() {
    let (g, _t) = deadlocked_with(5_000, "wo:7");
    assert_eq!(g.anchor_done(&admin(), "wo:7").unwrap(), 2);
    claim(&g, &bob(), "p").unwrap();
    assert_eq!(overwrites(&g), [event("p", "alice", "bob")]);
}

#[test]
fn a_session_end_or_anchor_release_of_a_member_outside_the_deadlock_is_no_yield() {
    let (g, _t) = deadlocked();
    claim(&g, &carol(), "z").unwrap();
    g.release_all(&carol()).unwrap();
    claim(&g, &bob(), "z").unwrap();
    assert!(overwrites(&g).is_empty());
}
