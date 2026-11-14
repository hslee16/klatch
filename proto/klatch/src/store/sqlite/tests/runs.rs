//! Deleting every room under a prefix (#122): the teardown of a run. Everything keyed by room goes, in every table,
//! and rooms that only share a leading part of the name stay.

use super::*;

fn populate(s: &SqliteStore, room: &str) {
    s.join(room, "alice", 1).unwrap();
    s.post(Seat::new(room, "alice"), "hello", 2).unwrap();
    s.claim(&ClaimRequest::new((room, "alice"), "src/a.rs", 600, "r"), 3)
        .unwrap();
    s.set_status(Seat::new(room, "alice"), "busy", 4).unwrap();
    s.set_room_state(room, Some("closed"), 5).unwrap();
    s.policy_set(&format!("label:{room}"), Some("private"), 6)
        .unwrap();
}

fn rooms_left(s: &SqliteStore) -> Vec<String> {
    let c = &s.conn;
    let mut names = std::collections::BTreeSet::new();
    let tables: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type='table'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|t| t.unwrap())
        .collect();
    for t in tables {
        let has_room = c
            .prepare(&format!("PRAGMA table_info({t})"))
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .any(|n| n.unwrap() == "room");
        if has_room && t != "refusals" {
            let rooms: Vec<String> = c
                .prepare(&format!("SELECT DISTINCT room FROM {t}"))
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .map(|x| x.unwrap())
                .collect();
            names.extend(rooms);
        }
    }
    names.into_iter().collect()
}

#[test]
fn deleting_a_prefix_removes_those_rooms_from_every_table_and_nothing_else() {
    let s = store();
    for room in [
        "run.a.main",
        "run.a.review",
        "run.ab.main",
        "run.b.main",
        "lobby",
    ] {
        populate(&s, room);
    }
    let gone = s.delete_rooms_with_prefix("run.a.", 10).unwrap();
    assert_eq!(gone.rooms, 2);
    assert_eq!(gone.messages, 2);
    assert_eq!(rooms_left(&s), vec!["lobby", "run.ab.main", "run.b.main"]);
    // the label policy of the gone rooms went too, the others kept theirs
    assert_eq!(s.policy_get("label:run.a.main").unwrap(), None);
    assert_eq!(
        s.policy_get("label:run.ab.main").unwrap().as_deref(),
        Some("private")
    );
    assert_eq!(s.claims("run.ab.main", 11).unwrap().len(), 1);
    assert_eq!(s.tail("run.ab.main", 0, 10).unwrap().len(), 1);
}

#[test]
fn a_prefix_is_matched_literally_so_wildcards_do_not_widen_it() {
    let s = store();
    populate(&s, "run.a.main");
    populate(&s, "runXaXmain");
    s.delete_rooms_with_prefix("run.a.", 10).unwrap();
    s.delete_rooms_with_prefix("run%", 10).unwrap();
    s.delete_rooms_with_prefix("run_", 10).unwrap();
    assert_eq!(rooms_left(&s), vec!["runXaXmain"]);
}

#[test]
fn an_empty_or_missing_prefix_deletes_nothing_and_the_refusal_record_is_kept() {
    let s = store();
    populate(&s, "run.a.main");
    s.record_refusal(
        &crate::store::NewRefusal::new("alice", "run.a.main", "post", "clearance"),
        7,
    )
    .unwrap();
    assert_eq!(s.delete_rooms_with_prefix("", 10).unwrap().rooms, 0);
    assert_eq!(s.delete_rooms_with_prefix("nothing.", 10).unwrap().rooms, 0);
    assert_eq!(rooms_left(&s), vec!["run.a.main"]);
    s.delete_rooms_with_prefix("run.a.", 10).unwrap();
    assert_eq!(
        s.refusals(10).unwrap().len(),
        1,
        "the record of a refusal is not part of a run"
    );
}
