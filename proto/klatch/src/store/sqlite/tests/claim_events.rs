//! The record of claim events (#1386): each grant, refusal, release, expiry and takeover leaves one small row of
//! counts and class, never a name; the record is pruned; a failure to record never fails the claim.

use super::*;
use crate::domain::Error;

/// `(ts, room, event, class, reason, held, waited)` of every row, oldest first.
type Row = (i64, String, String, String, String, i64, Option<i64>);

fn events(s: &SqliteStore) -> Vec<Row> {
    s.rows(
        "SELECT ts,room,event,class,reason,held,waited FROM claim_events ORDER BY id",
        [],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        },
    )
    .unwrap()
}

fn row(ts: i64, room: &str, (event, class, reason): (&str, &str, &str), held: i64) -> Row {
    (
        ts,
        room.into(),
        event.into(),
        class.into(),
        reason.into(),
        held,
        None,
    )
}

fn take(
    s: &SqliteStore,
    (who, path): (&str, &str),
    ttl: i64,
    now: i64,
) -> crate::domain::Result<()> {
    s.claim(&ClaimRequest::new(("r", who), path, ttl, ""), now)
}

#[test]
fn a_grant_leaves_one_row_with_the_class_and_a_renewal_leaves_none() {
    let s = store();
    take(&s, ("bob", "src/a.rs"), 600, 10).unwrap();
    take(&s, ("bob", "ticket:5"), 600, 11).unwrap();
    take(&s, ("bob", "ask:3"), 600, 12).unwrap();
    take(&s, ("bob", "merge:main"), 600, 13).unwrap();
    take(&s, ("bob", "src/a.rs"), 600, 100).unwrap(); // the holder renews
    assert_eq!(
        events(&s),
        vec![
            row(10, "r", ("granted", "file", ""), 0),
            row(11, "r", ("granted", "ticket", ""), 0),
            row(12, "r", ("granted", "ask", ""), 0),
            row(13, "r", ("granted", "other", ""), 0),
        ]
    );
}

#[test]
fn a_refusal_leaves_a_row_with_the_rule_word() {
    let s = store();
    take(&s, ("bob", "src"), 600, 0).unwrap();
    assert!(take(&s, ("alice", "src"), 600, 5).is_err());
    assert!(take(&s, ("alice", "src/a.rs"), 600, 6).is_err());
    s.claim(
        &ClaimRequest::new(("r", "carol"), "src/b.rs", 600, "").scoped("*"),
        7,
    )
    .unwrap_err();
    let refused: Vec<(String, String, String)> = events(&s)
        .into_iter()
        .filter(|e| e.2 == "refused")
        .map(|e| (e.3, e.4, e.1))
        .collect();
    assert_eq!(
        refused,
        vec![
            ("file".into(), "same-path".into(), "r".into()),
            ("file".into(), "overlap".into(), "r".into()),
            ("file".into(), "shared".into(), "r".into()),
        ]
    );
}

#[test]
fn a_deadlock_refusal_says_deadlock() {
    let s = store();
    take(&s, ("dave", "q"), 100, 0).unwrap();
    take(&s, ("bob", "p"), 100, 1).unwrap();
    take(&s, ("alice", "z"), 100, 2).unwrap();
    take(&s, ("alice", "q/x"), 100, 3).unwrap_err();
    take(&s, ("bob", "z"), 100, 4).unwrap_err();
    assert!(matches!(
        take(&s, ("alice", "p"), 100, 5),
        Err(Error::Conflict { deadlock: true, .. })
    ));
    let last = events(&s).pop().unwrap();
    assert_eq!((last.2.as_str(), last.4.as_str()), ("refused", "deadlock"));
}

#[test]
fn releases_say_who_released_and_how_long_the_claim_was_held() {
    let s = store();
    take(&s, ("bob", "a"), 1_000, 100).unwrap();
    s.release_at("r", "bob", "a", 160).unwrap();
    take(&s, ("bob", "ticket:1"), 1_000, 200).unwrap();
    s.release_with(&Release::new(("r", "bob"), "ticket:1", "done"), 290)
        .unwrap();
    take(&s, ("bob", "b"), 1_000, 300).unwrap();
    take(&s, ("bob", "ask:9"), 1_000, 300).unwrap();
    assert_eq!(s.release_member_claims("bob", 400).unwrap(), 2);
    s.claim(
        &ClaimRequest::new(("r", "carol"), "c", 1_000, "").anchored("gh:5"),
        500,
    )
    .unwrap();
    assert_eq!(s.release_anchored("gh:5", 530).unwrap().len(), 1);
    take(&s, ("dave", "d"), 1_000, 600).unwrap();
    assert_eq!(s.remove_member("r", "dave", 650).unwrap(), 1);
    let released: Vec<(String, String, i64)> = events(&s)
        .into_iter()
        .filter(|e| e.2 == "released")
        .map(|e| (e.3, e.4, e.5))
        .collect();
    assert_eq!(
        released,
        vec![
            ("file".into(), "holder".into(), 60),
            ("ticket".into(), "holder".into(), 90),
            ("file".into(), "session".into(), 100),
            ("ask".into(), "session".into(), 100),
            ("file".into(), "anchor".into(), 30),
            ("file".into(), "removed".into(), 50),
        ]
    );
}

#[test]
fn a_lapsed_lease_is_an_expiry_with_its_hold_time_and_a_new_holder_is_a_takeover() {
    let s = store();
    take(&s, ("bob", "src/a.rs"), 600, 1).unwrap();
    take(&s, ("alice", "src/a.rs"), 600, 700).unwrap();
    let got = events(&s);
    assert_eq!(
        got,
        vec![
            row(1, "r", ("granted", "file", ""), 0),
            row(700, "r", ("expired", "file", ""), 600),
            row(700, "r", ("granted", "file", ""), 0),
            row(700, "r", ("takeover", "file", ""), 600),
        ]
    );
}

#[test]
fn an_expiry_nobody_takes_over_is_an_expiry_only_and_each_lapse_is_one_row() {
    let s = store();
    take(&s, ("bob", "ticket:2"), 600, 0).unwrap();
    take(&s, ("carol", "other-file"), 600, 0).unwrap();
    take(&s, ("dave", "unrelated"), 600, 1_000).unwrap();
    take(&s, ("dave", "unrelated2"), 600, 1_001).unwrap();
    let expired: Vec<String> = events(&s)
        .into_iter()
        .filter(|e| e.2 == "expired")
        .map(|e| e.3)
        .collect();
    assert_eq!(expired, vec!["ticket", "file"]);
    assert!(events(&s).iter().all(|e| e.2 != "takeover"));
}

#[test]
fn a_conflict_that_ends_in_a_grant_records_the_wait_even_past_the_wait_window() {
    let s = store();
    take(&s, ("bob", "src/a.rs"), 5_000, 0).unwrap();
    take(&s, ("alice", "src/a.rs"), 600, 10).unwrap_err();
    take(&s, ("alice", "src/a.rs"), 600, 50).unwrap_err(); // the first refusal keeps its time
    s.release_at("r", "bob", "src/a.rs", 900).unwrap();
    take(&s, ("alice", "src/a.rs"), 600, 1_000).unwrap();
    let grant = events(&s).pop().unwrap();
    assert_eq!((grant.2.as_str(), grant.6), ("granted", Some(990)));
    // a later plain grant of the same member has no wait
    take(&s, ("alice", "x"), 600, 1_100).unwrap();
    assert_eq!(events(&s).pop().unwrap().6, None);
}

#[test]
fn an_old_row_is_pruned_by_age() {
    let s = store();
    take(&s, ("bob", "a"), 600, 0).unwrap();
    let late = super::super::claim_events::CLAIM_EVENT_MAX_AGE_SECS + 10;
    take(&s, ("bob", "b"), 600, late).unwrap();
    let got = events(&s);
    assert!(
        got.iter().all(|e| e.0 >= 10),
        "the day-zero rows are gone: {got:?}"
    );
    assert!(got.iter().any(|e| e.2 == "granted" && e.0 == late));
}

#[test]
fn the_table_is_capped_at_the_newest_rows() {
    let s = store();
    let cap = super::super::claim_events::CLAIM_EVENT_MAX_ROWS;
    s.conn
        .execute_batch(&format!(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<{}) \
             INSERT INTO claim_events(ts,room,event,class) SELECT 5,'r','granted','file' FROM n",
            cap + 50
        ))
        .unwrap();
    take(&s, ("bob", "a"), 600, 6).unwrap();
    let n: i64 = s
        .conn
        .query_row("SELECT COUNT(*) FROM claim_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, cap);
    assert_eq!(events(&s).pop().unwrap().0, 6, "the newest row is kept");
}

#[test]
fn a_recording_failure_never_fails_the_claim_and_is_counted() {
    let s = store();
    s.conn
        .execute_batch("DROP TABLE claim_events; DROP TABLE claim_conflicts_open;")
        .unwrap();
    take(&s, ("bob", "a"), 600, 0).unwrap();
    assert!(matches!(
        take(&s, ("alice", "a"), 600, 1),
        Err(Error::Conflict { .. })
    ));
    s.release_at("r", "bob", "a", 5).unwrap();
    take(&s, ("alice", "a"), 600, 6).unwrap();
    assert!(s.claim_record_errors() >= 4, "{}", s.claim_record_errors());
    assert_eq!(s.claims("r", 7).unwrap()[0].owner, "alice");
}

#[test]
fn no_claim_name_or_member_is_in_the_event_rows() {
    let s = store();
    take(&s, ("bob-SECRETMEMBER", "SECRETPATH/file.rs"), 600, 0).unwrap();
    take(&s, ("alice", "SECRETPATH/file.rs"), 600, 1).unwrap_err();
    s.release_at("r", "bob-SECRETMEMBER", "SECRETPATH/file.rs", 9)
        .unwrap();
    let all: String = s
        .rows(
            "SELECT room||'|'||event||'|'||class||'|'||reason FROM claim_events",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap()
        .join("\n");
    assert!(!all.contains("SECRET"), "{all}");
}

#[test]
fn passive_events_and_explicit_events_keep_their_own_caps() {
    let s = store();
    let cap = super::super::claim_events::PASSIVE_EVENT_MAX_ROWS;
    // a flood of passive renewals (#1401) must not push the explicit history out
    s.conn
        .execute_batch(&format!(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<{}) \
             INSERT INTO claim_events(ts,room,event,class) SELECT 5,'r','passive_renewed','file' FROM n",
            cap + 50
        ))
        .unwrap();
    take(&s, ("bob", "a"), 600, 6).unwrap();
    let count = |like: &str| -> i64 {
        s.conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM claim_events WHERE event {like} 'passive\\_%' ESCAPE '\\'"
                ),
                [],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(count("NOT LIKE"), 1, "the explicit grant is kept");
    s.take_passive_claim(
        &ClaimRequest::new(("r", "bob"), "b", 300, "passive: edited (klatch hook)"),
        7,
    )
    .unwrap();
    assert_eq!(
        count("LIKE"),
        cap,
        "the passive events are capped on their own"
    );
    assert_eq!(count("NOT LIKE"), 1);
}

#[test]
fn a_failed_insert_in_a_sweep_batch_is_counted_and_the_rest_is_still_recorded() {
    // #1702: one bad row must not drop the events after it
    let s = store();
    s.conn
        .execute_batch(
            "CREATE TRIGGER refuse_bad BEFORE INSERT ON claim_events WHEN NEW.room='bad'
             BEGIN SELECT RAISE(ABORT,'refused'); END;",
        )
        .unwrap();
    let ev = |room| crate::store::sqlite::claim_events::ClaimEvent {
        room,
        path: "p",
        event: "expired",
        reason: "",
        held: 1,
        waited: None,
    };
    s.record_claim_events(10, &[ev("a"), ev("bad"), ev("b"), ev("bad"), ev("c")]);
    let rooms: Vec<String> = events(&s).into_iter().map(|e| e.1).collect();
    assert_eq!(rooms, ["a", "b", "c"]);
    assert_eq!(s.claim_record_errors(), 2, "each failed insert counts once");
}
