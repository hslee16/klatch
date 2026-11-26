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
        m.contains("carol, dave") && m.contains("your path a"),
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
