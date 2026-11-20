//! The review queue (#48): a post that needs review waits here until a judge returns a verdict.

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
