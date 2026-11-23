//! The machine checkpoint in the store (#1408): one per room and member, replaced and never appended; the trail
//! between hooks; and what pruning and removal do with them.

use super::*;
use crate::domain::Trail;

const SECRET: &str = "SENTINEL-sk-live-4242";

#[test]
fn a_checkpoint_is_replaced_not_appended_and_says_whether_it_replaced_one() {
    let s = store();
    assert!(
        !s.put_machine_checkpoint(("r", "alice"), "first", false, 10)
            .unwrap()
    );
    assert!(
        s.put_machine_checkpoint(("r", "alice"), "second", true, 20)
            .unwrap()
    );
    assert!(
        !s.put_machine_checkpoint(("other", "alice"), "elsewhere", false, 21)
            .unwrap()
    );
    let in_r = s.machine_checkpoints("r").unwrap();
    assert_eq!(in_r.len(), 1, "one row per room and member");
    assert_eq!(
        (in_r[0].body.as_str(), in_r[0].ts, in_r[0].last),
        ("second", 20, true)
    );
    assert_eq!(s.machine_checkpoints("other").unwrap().len(), 1);
}

#[test]
fn a_trail_round_trips_and_clears() {
    let s = store();
    assert_eq!(s.trail_of("alice").unwrap(), Trail::default());
    let t = Trail {
        files: vec!["src/a.rs".into(), "dir with space/b.rs".into()],
        tools: vec!["edit".into(), "run".into()],
    };
    s.save_trail("alice", &t, 5).unwrap();
    assert_eq!(s.trail_of("alice").unwrap(), t);
    assert_eq!(s.trail_of("bob").unwrap(), Trail::default());
    s.clear_trail("alice").unwrap();
    assert_eq!(s.trail_of("alice").unwrap(), Trail::default());
}

#[test]
fn the_rows_hold_only_what_the_gateway_handed_them() {
    // the store keeps what it is given and nothing besides: scan every column of every new table for the sentinel
    let s = store();
    s.save_trail(
        "alice",
        &Trail {
            files: vec!["src/a.rs".into()],
            tools: vec!["edit".into(), "run".into()],
        },
        5,
    )
    .unwrap();
    s.put_machine_checkpoint(
        ("r", "alice"),
        "turn ended; files src/a.rs; tools edit run",
        false,
        6,
    )
    .unwrap();
    for table in [
        "member_trail",
        "machine_checkpoints",
        "machine_checkpoint_events",
    ] {
        let mut st = s.conn.prepare(&format!("SELECT * FROM {table}")).unwrap();
        let cols = st.column_count();
        let mut rows = st.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            for i in 0..cols {
                let v: rusqlite::types::Value = row.get(i).unwrap();
                assert!(!format!("{v:?}").contains(SECRET), "{table} column {i}");
            }
        }
    }
    let events: String = s
        .conn
        .query_row(
            "SELECT group_concat(name) FROM pragma_table_info('machine_checkpoint_events')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        events
            .split(',')
            .all(|c| ["id", "ts", "room", "replaced", "last", "bytes"].contains(&c)),
        "the event record has no text column: {events}"
    );
}

#[test]
fn pruning_spares_a_member_with_a_recent_checkpoint_and_then_takes_its_rows() {
    let s = store();
    s.join("r", "alice", 1).unwrap();
    s.join("r", "bob", 1).unwrap();
    s.put_machine_checkpoint(("r", "alice"), "turn ended", true, 900)
        .unwrap();
    s.save_trail(
        "bob",
        &Trail {
            files: vec!["a".into()],
            tools: vec![],
        },
        1,
    )
    .unwrap();
    // both have been gone past the grace period; alice left a checkpoint within the hold
    s.prune_members(100, 1000).unwrap();
    assert_eq!(
        s.machine_checkpoints("r").unwrap().len(),
        1,
        "alice may come back"
    );
    assert_eq!(
        s.trail_of("bob").unwrap(),
        Trail::default(),
        "bob is pruned with his trail"
    );
    // a week and more later nothing holds her
    s.prune_members(100, 900 + crate::domain::TASK_HOLD_SECS + 10)
        .unwrap();
    assert!(s.machine_checkpoints("r").unwrap().is_empty());
}

#[test]
fn removing_a_member_from_a_room_takes_its_checkpoint_there_only() {
    let s = store();
    s.join("r", "alice", 1).unwrap();
    s.join("q", "alice", 1).unwrap();
    s.put_machine_checkpoint(("r", "alice"), "x", false, 5)
        .unwrap();
    s.put_machine_checkpoint(("q", "alice"), "x", false, 5)
        .unwrap();
    s.remove_member("r", "alice", 10).unwrap();
    assert!(s.machine_checkpoints("r").unwrap().is_empty());
    assert_eq!(s.machine_checkpoints("q").unwrap().len(), 1);
}
