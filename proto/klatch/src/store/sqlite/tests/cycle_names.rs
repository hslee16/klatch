//! #1603: a deadlock refusal names the path of the refused member that closes the cycle and the members on the chain,
//! so a false refusal can be checked. It names only what the refused member can already see: its own held path, and
//! members of this room that wait on the chain; never another room's members or paths, nor members off the chain.

use super::*;
use crate::domain::Error;

fn take_in(
    s: &SqliteStore,
    room: &str,
    who: &str,
    path: &str,
    now: i64,
) -> crate::domain::Result<()> {
    s.claim(&ClaimRequest::new((room, who), path, 1_000, ""), now)
}

fn take(s: &SqliteStore, who: &str, path: &str, now: i64) -> crate::domain::Result<()> {
    take_in(s, "r", who, path, now)
}

fn conflict(r: crate::domain::Result<()>) -> Error {
    r.unwrap_err()
}

#[test]
fn a_two_member_cycle_names_the_requesters_path_and_the_holder() {
    let s = store();
    take(&s, "alice", "f", 0).unwrap();
    take(&s, "bob", "g", 0).unwrap();
    assert!(take(&s, "bob", "f", 1).is_err());
    let e = conflict(take(&s, "alice", "g", 2));
    match &e {
        Error::Conflict {
            deadlock: true,
            cycle,
            via,
            ..
        } => {
            assert_eq!(cycle, &["bob".to_string()]);
            assert_eq!(via, "f");
        }
        other => panic!("{other:?}"),
    }
    let m = e.to_string();
    assert!(m.contains("your path f"), "{m}");
    assert!(m.contains("bob"), "{m}");
}

#[test]
fn a_three_member_chain_names_every_member_in_order() {
    // alice asks c (carol holds). carol waits for b (dave holds). dave waits for a (alice holds).
    let s = store();
    take(&s, "alice", "a", 0).unwrap();
    take(&s, "carol", "c", 0).unwrap();
    take(&s, "dave", "b", 0).unwrap();
    assert!(take(&s, "carol", "b", 1).is_err());
    assert!(take(&s, "dave", "a", 2).is_err());
    let e = conflict(take(&s, "alice", "c", 3));
    match &e {
        Error::Conflict {
            deadlock: true,
            cycle,
            via,
            ..
        } => {
            assert_eq!(cycle, &["carol".to_string(), "dave".to_string()]);
            assert_eq!(via, "a");
        }
        other => panic!("{other:?}"),
    }
    let m = e.to_string();
    assert!(
        m.contains("carol waits through dave for your path a"),
        "{m}"
    );
}

#[test]
fn members_off_the_chain_and_other_rooms_are_never_named() {
    let s = store();
    take(&s, "alice", "f", 0).unwrap();
    take(&s, "bob", "g", 0).unwrap();
    assert!(take(&s, "bob", "f", 1).is_err());
    // erin waits on something unrelated in this room; zed and a path sit in another room.
    take(&s, "frank", "z", 0).unwrap();
    assert!(take(&s, "erin", "z", 1).is_err());
    take_in(&s, "other", "zed", "secret/path", 0).unwrap();
    take_in(&s, "other", "alice", "g", 0).unwrap();
    assert!(take_in(&s, "other", "zed", "g", 1).is_err());
    let m = conflict(take(&s, "alice", "g", 2)).to_string();
    for leak in ["erin", "frank", "zed", "secret", "z,"] {
        assert!(!m.contains(leak), "{leak} leaked: {m}");
    }
}

#[test]
fn a_plain_refusal_names_no_cycle() {
    let s = store();
    take(&s, "alice", "f", 0).unwrap();
    take(&s, "bob", "g", 0).unwrap();
    let e = conflict(take(&s, "alice", "g", 2));
    match &e {
        Error::Conflict {
            deadlock: false,
            cycle,
            via,
            ..
        } => assert!(cycle.is_empty() && via.is_empty()),
        other => panic!("{other:?}"),
    }
    assert!(!e.to_string().contains("your path"), "{e}");
}

fn parts(e: &Error) -> (Vec<String>, String) {
    match e {
        Error::Conflict {
            deadlock: true,
            cycle,
            via,
            ..
        } => (cycle.clone(), via.clone()),
        other => panic!("not a deadlock: {other:?}"),
    }
}

#[test]
fn the_text_names_a_member_that_is_in_the_cycle_when_the_first_blocker_is_not() {
    // carol holds src/x and waits for nothing; bob holds src/y and waits for f; alice holds f and claims src.
    let s = store();
    take(&s, "carol", "src/x", 0).unwrap();
    take(&s, "bob", "src/y", 0).unwrap();
    take(&s, "alice", "f", 0).unwrap();
    assert!(take(&s, "bob", "f", 1).is_err());
    let e = conflict(take(&s, "alice", "src", 2));
    let (cycle, via) = parts(&e);
    assert_eq!((cycle, via.as_str()), (vec!["bob".to_string()], "f"));
    let m = e.to_string();
    assert!(
        m.contains("deadlock: bob is waiting") && m.contains("the cycle: bob waits"),
        "{m}"
    );
    assert!(
        !m.contains("carol is waiting") && !m.contains("carol waits"),
        "{m}"
    );
}

#[test]
fn a_two_member_cycle_does_not_read_as_the_holder_waiting_through_itself() {
    let s = store();
    take(&s, "alice", "f", 0).unwrap();
    take(&s, "bob", "g", 0).unwrap();
    assert!(take(&s, "bob", "f", 1).is_err());
    let m = conflict(take(&s, "alice", "g", 2)).to_string();
    assert!(!m.contains("through bob"), "{m}");
    assert!(m.contains("bob waits for your path f"), "{m}");
}

#[test]
fn a_long_chain_shows_the_first_names_and_counts_the_rest() {
    let s = store();
    take(&s, "alice", "a", 0).unwrap();
    let n = 30;
    for i in 0..n {
        take(&s, &format!("m{i}"), &format!("p{i}"), 0).unwrap();
    }
    for i in 0..n - 1 {
        assert!(take(&s, &format!("m{i}"), &format!("p{}", i + 1), 1).is_err());
    }
    assert!(take(&s, &format!("m{}", n - 1), "a", 1).is_err());
    let e = conflict(take(&s, "alice", "p0", 2));
    let (cycle, via) = parts(&e);
    assert_eq!(cycle.len(), n);
    assert_eq!(via, "a");
    let m = e.to_string();
    assert!(m.contains("m0 waits through m1, m2"), "{m}");
    assert!(m.contains("and 2") && m.contains("more"), "{m}");
    assert!(!m.contains("m29") && m.len() < 700, "{} {m}", m.len());
}

#[test]
fn via_is_the_requesters_own_path_when_the_last_member_waits_for_several() {
    // bob waits for k (carol's, held first) and f (alice's); only f is alice's.
    let s = store();
    take(&s, "carol", "k", 0).unwrap();
    take(&s, "alice", "f", 0).unwrap();
    take(&s, "bob", "g", 0).unwrap();
    assert!(take(&s, "bob", "k", 1).is_err());
    assert!(take(&s, "bob", "f", 2).is_err());
    let e = conflict(take(&s, "alice", "g", 3));
    let (cycle, via) = parts(&e);
    assert_eq!((cycle, via.as_str()), (vec!["bob".to_string()], "f"));
    assert!(!e.to_string().contains("path k"), "{e}");
}

#[test]
fn no_other_path_of_the_room_appears_in_the_via() {
    let s = store();
    take(&s, "alice", "f", 0).unwrap();
    take(&s, "bob", "g", 0).unwrap();
    assert!(take(&s, "bob", "f", 1).is_err());
    take(&s, "frank", "z", 0).unwrap();
    assert!(take(&s, "bob", "z", 2).is_err());
    let (_, via) = parts(&conflict(take(&s, "alice", "g", 3)));
    assert_eq!(via, "f");
}
