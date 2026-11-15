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
    assert!(s.resolve_review(a, "allow", "", 20).unwrap());
    let left = s.pending_reviews(10).unwrap();
    assert_eq!(left.iter().map(|q| q.id).collect::<Vec<_>>(), vec![b]);
}

#[test]
fn a_verdict_is_recorded_once() {
    let s = store();
    let a = s.enqueue_review(&item("r", 1), 10).unwrap();
    assert!(s.resolve_review(a, "deny", "scrubber hit", 20).unwrap());
    assert!(
        !s.resolve_review(a, "allow", "", 21).unwrap(),
        "a second verdict replaced the first"
    );
    assert!(!s.resolve_review(9999, "allow", "", 21).unwrap());
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
