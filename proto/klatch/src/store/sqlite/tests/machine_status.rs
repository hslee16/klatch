//! The machine status (#1403) in the store: a status is written only over nothing or a text the machine owns, a repeat
//! waits for the window, and every write is counted by class word with no member and no text.

use super::*;
use crate::store::Seat;

const OWNED: &[&str] = &[
    "working (auto)",
    "idle (auto)",
    "parked (auto)",
    "done session ended",
];

fn put(s: &SqliteStore, member: &str, class: &str, now: i64) -> bool {
    let text = format!("{class} (auto)");
    s.put_machine_status(Seat::new("r", member), (&text, class), OWNED, (now, 60))
        .unwrap()
}

fn status(s: &SqliteStore, member: &str) -> Option<String> {
    s.members("r")
        .unwrap()
        .into_iter()
        .find(|m| m.name == member)
        .and_then(|m| m.status)
}

#[test]
fn it_writes_over_nothing_and_over_its_own_text_and_never_over_the_members() {
    let s = store();
    s.join("r", "alice", 1).unwrap();
    s.join("r", "bob", 1).unwrap();
    assert!(put(&s, "alice", "working", 10));
    assert!(put(&s, "alice", "idle", 11));
    s.set_status(Seat::new("r", "bob"), "parked: CI, back by 2h", 12)
        .unwrap();
    assert!(!put(&s, "bob", "working", 13));
    assert_eq!(status(&s, "bob").as_deref(), Some("parked: CI, back by 2h"));
    s.set_status(Seat::new("r", "alice"), "done session ended", 14)
        .unwrap();
    assert!(
        put(&s, "alice", "working", 15),
        "the session end text is the machine's"
    );
}

#[test]
fn a_repeat_waits_for_the_window_and_a_stranger_is_not_joined() {
    let s = store();
    s.join("r", "alice", 1).unwrap();
    assert!(put(&s, "alice", "working", 100));
    assert!(!put(&s, "alice", "working", 159));
    assert!(put(&s, "alice", "working", 160));
    assert!(!put(&s, "nobody", "working", 200));
    assert_eq!(s.members("r").unwrap().len(), 1);
}

#[test]
fn the_counts_hold_a_class_a_room_and_a_time_and_nothing_else() {
    let s = store();
    s.join("r", "alice-TOPSECRET", 1).unwrap();
    put(&s, "alice-TOPSECRET", "working", 10);
    put(&s, "alice-TOPSECRET", "idle", 20);
    put(&s, "alice-TOPSECRET", "parked", 30);
    put(&s, "alice-TOPSECRET", "idle", 40);
    let mut st = s
        .conn
        .prepare("SELECT * FROM machine_status_events")
        .unwrap();
    let cols = st.column_count();
    let mut rows = st.query([]).unwrap();
    let mut n = 0;
    while let Some(row) = rows.next().unwrap() {
        n += 1;
        for i in 0..cols {
            let v: rusqlite::types::Value = row.get(i).unwrap();
            assert!(!format!("{v:?}").contains("TOPSECRET"), "column {i}");
        }
    }
    assert_eq!(n, 4);
}
