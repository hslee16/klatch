//! The review queue (#48): a post that needs review waits here until a judge returns a verdict.

use super::*;
use crate::store::NewReview;

fn item<'a>(room: &'a str, seq: i64) -> NewReview<'a> {
    NewReview::new(room, seq, "public", "crosses to public")
}

#[test]
fn queued_items_come_back_oldest_first_and_only_while_pending() {
    let s = store();
    let a = s.enqueue_review(&item("r", 1), 10).unwrap();
    let b = s.enqueue_review(&item("r", 2), 11).unwrap();
    let pending = s.pending_reviews(10).unwrap();
    assert_eq!(pending.iter().map(|q| q.id).collect::<Vec<_>>(), vec![a, b]);
    assert_eq!(
        (
            pending[0].room.as_str(),
            pending[0].seq,
            pending[0].to_label.as_str()
        ),
        ("r", 1, "public")
    );
    assert!(
        s.resolve_review(&ReviewResolution::new(a, "allow", ""), 20)
            .unwrap()
    );
    let left = s.pending_reviews(10).unwrap();
    assert_eq!(left.iter().map(|q| q.id).collect::<Vec<_>>(), vec![b]);
}

#[test]
fn a_verdict_is_recorded_once() {
    let s = store();
    let a = s.enqueue_review(&item("r", 1), 10).unwrap();
    assert!(
        s.resolve_review(&ReviewResolution::new(a, "deny", "scrubber hit"), 20)
            .unwrap()
    );
    assert!(
        !s.resolve_review(&ReviewResolution::new(a, "allow", ""), 21)
            .unwrap(),
        "a second verdict replaced the first"
    );
    assert!(
        !s.resolve_review(&ReviewResolution::new(9999, "allow", ""), 21)
            .unwrap()
    );
}

#[test]
fn the_limit_bounds_one_drain_and_the_same_message_queues_once_per_label() {
    let s = store();
    for seq in 1..=5 {
        s.enqueue_review(&item("r", seq), 10).unwrap();
    }
    assert_eq!(s.pending_reviews(2).unwrap().len(), 2);
    let again = s.enqueue_review(&item("r", 1), 12).unwrap();
    let first = s.pending_reviews(1).unwrap()[0].id;
    assert_eq!(
        again, first,
        "a message already waiting for that label was queued twice"
    );
    assert_eq!(s.pending_reviews(10).unwrap().len(), 5);
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
fn a_crossing_item_keeps_its_target_requester_and_outcome() {
    let s = store();
    let it = NewReview::new("r", 1, "public", "crossing").crossing("open", "alice");
    let id = s.enqueue_review(&it, 10).unwrap();
    let q = &s.pending_reviews(10).unwrap()[0];
    assert_eq!(
        (q.to_room.as_str(), q.requester.as_str()),
        ("open", "alice")
    );
    s.set_review_text(id, "[x]").unwrap();
    assert!(
        s.resolve_review(&ReviewResolution::new(id, "redact", ""), 20)
            .unwrap()
    );
    let due = s.unapplied_reviews(10).unwrap();
    assert_eq!((due.len(), due[0].redacted.as_str()), (1, "[x]"));
    s.mark_review_applied(id, "released").unwrap();
    assert!(s.unapplied_reviews(10).unwrap().is_empty());
    assert_eq!(s.reviews(10).unwrap()[0].applied, "released");
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
