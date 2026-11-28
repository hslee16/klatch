//! The review queue (#48): a post that needs review waits here until a judge returns a verdict.

use super::super::review::{
    PENDING_TAIL, WAITING_COUNT_SQL, WAITING_SAME_SQL, pending_after_tail, review_select,
};
use super::*;
use crate::store::NewReview;

fn item<'a>(room: &'a str, seq: i64) -> NewReview<'a> {
    NewReview::new(room, seq, "public", "crosses to public")
}

#[test]
fn a_database_from_before_the_queue_gains_the_table() {
    let path = temp_db("review-migrate");
    drop(SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap());
    {
        let c = keyed(&path);
        c.execute_batch("DROP TABLE review_queue; PRAGMA user_version = 22;")
            .unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    assert!(s.pending_reviews(10).unwrap().is_empty());
    s.enqueue_review(&item("r", 1), 1).unwrap();
}

#[test]
fn a_version_23_queue_gains_the_crossing_columns() {
    let path = temp_db("review-migrate-23");
    drop(SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap());
    {
        let c = keyed(&path);
        c.execute_batch(
            "DROP TABLE review_queue; CREATE TABLE review_queue(id INTEGER PRIMARY KEY AUTOINCREMENT, room TEXT NOT NULL, seq INTEGER NOT NULL, to_label TEXT NOT NULL, reason TEXT NOT NULL, ts INTEGER NOT NULL, verdict TEXT, detail TEXT, resolved INTEGER); INSERT INTO review_queue(room,seq,to_label,reason,ts) VALUES('r',1,'public','old',1); PRAGMA user_version = 23;",
        )
        .unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    let q = &s.pending_reviews(10).unwrap()[0];
    assert_eq!((q.reason.as_str(), q.to_room.as_str()), ("old", ""));
}

#[test]
fn a_version_33_queue_gains_told_and_old_outcomes_are_not_told_again() {
    let path = temp_db("review-migrate-33");
    drop(SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap());
    {
        let c = keyed(&path);
        c.execute_batch(
            "DROP TABLE review_queue; CREATE TABLE review_queue(id INTEGER PRIMARY KEY AUTOINCREMENT, room TEXT NOT NULL, seq INTEGER NOT NULL, to_label TEXT NOT NULL, reason TEXT NOT NULL, ts INTEGER NOT NULL, verdict TEXT, detail TEXT, resolved INTEGER, to_room TEXT NOT NULL DEFAULT '', requester TEXT NOT NULL DEFAULT '', redacted TEXT NOT NULL DEFAULT '', applied TEXT NOT NULL DEFAULT ''); INSERT INTO review_queue(room,seq,to_label,reason,ts,to_room,requester,verdict,applied) VALUES('v',1,'public','crossing',1,'o','alice','allow','released'); INSERT INTO review_queue(room,seq,to_label,reason,ts,to_room,requester) VALUES('v',2,'public','crossing',2,'o','alice'); PRAGMA user_version = 33;",
        )
        .unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    let told = s.crossing_notices("v", "alice").unwrap();
    assert_eq!(told.len(), 1, "only the waiting row has news: {told:?}");
    assert_eq!(told[0].seq, 2);
    assert!(s.crossing_notices("v", "alice").unwrap().is_empty());
}

#[test]
fn concurrent_connections_of_one_requester_cannot_exceed_the_cap() {
    // #1736: the count and the insert are one transaction, so exactly `cap` of the racing connections are accepted
    const CAP: usize = 5;
    for round in 0..10 {
        let path = temp_db(&format!("cap-race{round}"));
        let key = crate::store::insecure_test_key;
        drop(SqliteStore::open(&path, &key()).unwrap());
        let gate = std::sync::Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8i64)
            .map(|t| {
                let (path, gate) = (path.to_path_buf(), gate.clone());
                std::thread::spawn(move || {
                    let s = SqliteStore::open(&path, &key()).unwrap();
                    let it =
                        NewReview::new("r", t + 1, "public", "crossing").crossing("open", "alice");
                    gate.wait();
                    s.enqueue_review_capped(&it, 1, CAP).unwrap().is_some()
                })
            })
            .collect();
        let accepted = handles
            .into_iter()
            .filter(|_| true)
            .map(|h| h.join().unwrap())
            .filter(|ok| *ok)
            .count();
        let s = SqliteStore::open(&path, &key()).unwrap();
        assert_eq!(
            (accepted, s.waiting_crossings_of("alice").unwrap()),
            (CAP, CAP),
            "round {round}"
        );
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}

/// The query plan of `sql` with the given parameters, one line per step.
fn plan_of(s: &SqliteStore, sql: &str, n_params: usize) -> String {
    let mut stmt = s
        .conn
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap();
    let params: Vec<rusqlite::types::Value> = (0..n_params).map(|_| 1i64.into()).collect();
    let rows: Vec<String> = stmt
        .query_map(rusqlite::params_from_iter(params), |r| {
            r.get::<_, String>(3)
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    rows.join(" | ")
}

#[test]
fn the_dedupe_and_cap_lookups_of_an_enqueue_search_an_index_and_never_scan_the_queue() {
    let s = store();
    for (what, sql, n) in [
        ("dedupe", WAITING_SAME_SQL.to_string(), 5),
        ("count", WAITING_COUNT_SQL.to_string(), 1),
    ] {
        let plan = plan_of(&s, &sql, n);
        assert!(
            plan.contains("review_queue_waiting_requester"),
            "{what}: {plan}"
        );
        assert!(!plan.contains("SCAN"), "{what}: {plan}");
    }
}

#[test]
fn the_pages_of_the_waiting_queue_search_the_waiting_index_and_never_walk_resolved_items() {
    let s = store();
    for (what, tail) in [
        ("first page", PENDING_TAIL.to_string()),
        ("later page", pending_after_tail(7)),
    ] {
        let plan = plan_of(&s, &review_select(&tail), 1);
        assert!(plan.contains("review_queue_waiting"), "{what}: {plan}");
        assert!(plan.contains("USING INDEX"), "{what}: {plan}");
        assert!(
            !plan.contains("TEMP B-TREE"),
            "{what}: sorted from the index: {plan}"
        );
    }
}

#[test]
fn a_database_from_before_the_review_indexes_gains_them_and_keeps_its_queue() {
    let path = temp_db("review-migrate-indexes");
    {
        let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
        s.enqueue_review(
            &NewReview::new("v", 1, "public", "crossing").crossing("open", "alice"),
            5,
        )
        .unwrap();
        s.conn
            .execute_batch(
                "DROP INDEX review_queue_waiting_requester; DROP INDEX review_queue_waiting; PRAGMA user_version = 45;",
            )
            .unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    let n: i64 = s
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name IN ('review_queue_waiting_requester','review_queue_waiting')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 2);
    assert_eq!(s.waiting_crossings_of("alice").unwrap(), 1);
    assert_eq!(s.pending_reviews(10).unwrap().len(), 1);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}
