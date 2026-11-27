//! The review queue (#48, #497, #679), written against the trait only (#956). `Feature::Reviews` runs these, so a store
//! that declares the queue is backed by the same checks as SQLite.
use super::*;

fn item<'a>(room: &'a str, seq: i64) -> NewReview<'a> {
    NewReview::new(room, seq, "public", "crosses to public")
}

fn ids(items: &[QueuedReview]) -> Vec<i64> {
    items.iter().map(|q| q.id).collect()
}

pub fn queued_items_come_back_oldest_first_and_only_while_pending<S: Store>(make: fn() -> S) {
    let s = make();
    let a = s.enqueue_review(&item("r", 1), 10).unwrap();
    let b = s.enqueue_review(&item("r", 2), 11).unwrap();
    let pending = s.pending_reviews(10).unwrap();
    assert_eq!(ids(&pending), vec![a, b]);
    assert_eq!(
        (
            pending[0].room.as_str(),
            pending[0].seq,
            pending[0].to_label.as_str(),
            pending[0].reason.as_str()
        ),
        ("r", 1, "public", "crosses to public")
    );
    assert!(
        s.resolve_review(&ReviewResolution::new(a, "allow", ""), 20)
            .unwrap()
    );
    assert_eq!(ids(&s.pending_reviews(10).unwrap()), vec![b]);
}

pub fn a_verdict_is_recorded_once<S: Store>(make: fn() -> S) {
    let s = make();
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
            .unwrap(),
        "a verdict on an unknown item was accepted"
    );
    let listed = s.reviews(10).unwrap();
    assert_eq!(
        (listed[0].verdict.as_str(), listed[0].detail.as_str()),
        ("deny", "scrubber hit"),
        "the first verdict and its detail are kept"
    );
}

pub fn the_limit_bounds_one_drain_and_the_same_message_queues_once_per_label<S: Store>(
    make: fn() -> S,
) {
    let s = make();
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
    let other = s
        .enqueue_review(&NewReview::new("r", 1, "partner", "x"), 13)
        .unwrap();
    assert_ne!(other, first, "another label shares the first label's item");
    assert_eq!(s.pending_reviews(10).unwrap().len(), 6);
}

pub fn the_listing_is_newest_first_and_keeps_resolved_items<S: Store>(make: fn() -> S) {
    let s = make();
    let a = s.enqueue_review(&item("r", 1), 10).unwrap();
    let b = s.enqueue_review(&item("r", 2), 11).unwrap();
    s.resolve_review(&ReviewResolution::new(a, "allow", ""), 20)
        .unwrap();
    assert_eq!(ids(&s.reviews(10).unwrap()), vec![b, a]);
    assert_eq!(ids(&s.reviews(1).unwrap()), vec![b], "the limit applies");
}

pub fn a_crossing_item_keeps_its_target_requester_and_outcome<S: Store>(make: fn() -> S) {
    let s = make();
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

pub fn a_deny_or_a_plain_item_is_never_due_to_be_applied<S: Store>(make: fn() -> S) {
    let s = make();
    let plain = s.enqueue_review(&item("r", 1), 10).unwrap();
    let denied = s
        .enqueue_review(
            &NewReview::new("r", 2, "public", "c").crossing("open", "alice"),
            10,
        )
        .unwrap();
    s.resolve_review(&ReviewResolution::new(plain, "allow", ""), 20)
        .unwrap();
    s.resolve_review(&ReviewResolution::new(denied, "deny", ""), 20)
        .unwrap();
    assert!(
        s.unapplied_reviews(10).unwrap().is_empty(),
        "only an allow or redact verdict on a crossing waits to be applied"
    );
}

pub fn a_crossing_is_told_to_its_requester_once_while_waiting_and_once_for_its_outcome<S: Store>(
    make: fn() -> S,
) {
    let s = make();
    let id = s
        .enqueue_review(
            &NewReview::new("v", 1, "public", "c").crossing("open", "alice"),
            10,
        )
        .unwrap();
    let other = s
        .enqueue_review(
            &NewReview::new("v", 2, "public", "c").crossing("open", "bob"),
            10,
        )
        .unwrap();
    assert_eq!(
        ids(&s.crossing_notices("v", "alice").unwrap()),
        vec![id],
        "the waiting notice, for the requester only"
    );
    assert!(
        s.crossing_notices("v", "alice").unwrap().is_empty(),
        "the waiting notice repeated"
    );
    assert!(
        s.crossing_notices("elsewhere", "alice").unwrap().is_empty(),
        "a notice leaked into another room"
    );
    s.resolve_review(&ReviewResolution::new(id, "deny", "no"), 20)
        .unwrap();
    let told = s.crossing_notices("v", "alice").unwrap();
    assert_eq!(ids(&told), vec![id], "the outcome notice");
    assert_eq!(told[0].verdict, "deny");
    assert!(s.crossing_notices("v", "alice").unwrap().is_empty());
    assert_eq!(ids(&s.crossing_notices("v", "bob").unwrap()), vec![other]);
}

pub fn pending_pages_by_cursor_and_waiting_crossings_are_counted_per_requester<S: Store>(
    make: fn() -> S,
) {
    let s = make();
    let cross = |seq: i64, who: &str| {
        let it = NewReview::new("r", seq, "public", "crossing").crossing("open", who);
        s.enqueue_review(&it, 10).unwrap()
    };
    let a1 = cross(1, "alice");
    let b1 = cross(2, "bob");
    let a2 = cross(3, "alice");
    let a3 = cross(4, "alice");
    assert_eq!(ids(&s.pending_reviews_after(0, 2).unwrap()), vec![a1, b1]);
    assert_eq!(ids(&s.pending_reviews_after(b1, 2).unwrap()), vec![a2, a3]);
    assert!(s.pending_reviews_after(a3, 2).unwrap().is_empty());
    assert_eq!(s.waiting_crossings_of("alice").unwrap(), 3);
    assert_eq!(s.waiting_crossings_of("bob").unwrap(), 1);
    assert_eq!(s.waiting_crossings_of("carol").unwrap(), 0);
    s.resolve_review(&ReviewResolution::new(a1, "deny", ""), 20)
        .unwrap();
    assert_eq!(s.waiting_crossings_of("alice").unwrap(), 2);
    // a plain item (no target room) is no crossing
    s.enqueue_review(&item("r", 9), 11).unwrap();
    assert_eq!(s.waiting_crossings_of("").unwrap(), 0);
}

pub fn a_flag_is_kept_on_a_waiting_item_and_a_verdict_replaces_it<S: Store>(make: fn() -> S) {
    let s = make();
    let id = s.enqueue_review(&item("v", 1), 10).unwrap();
    s.set_review_flag(id, "screen:injection").unwrap();
    s.set_review_flag(id, "screen:injection").unwrap();
    let waiting = &s.pending_reviews(10).unwrap()[0];
    assert_eq!(waiting.detail, "screen:injection", "the flag is kept");
    assert!(waiting.verdict.is_empty(), "a flag is not a verdict");
    assert!(
        s.resolve_review(&ReviewResolution::new(id, "deny", "by r: no"), 20)
            .unwrap()
    );
    s.set_review_flag(id, "late").unwrap();
    assert_eq!(
        s.reviews(10).unwrap()[0].detail,
        "by r: no",
        "a flag on a resolved item changes nothing"
    );
}

pub fn a_capped_enqueue_stops_at_the_requesters_cap_and_returns_a_waiting_duplicate<S: Store>(
    make: fn() -> S,
) {
    let s = make();
    fn cross(seq: i64, who: &str) -> NewReview<'_> {
        NewReview::new("r", seq, "public", "crossing").crossing("open", who)
    }
    let a1 = s
        .enqueue_review_capped(&cross(1, "alice"), 10, 2)
        .unwrap()
        .unwrap();
    s.enqueue_review_capped(&cross(2, "alice"), 10, 2)
        .unwrap()
        .unwrap();
    assert_eq!(
        s.enqueue_review_capped(&cross(3, "alice"), 10, 2).unwrap(),
        None,
        "a third waiting crossing was queued past the cap of 2"
    );
    assert_eq!(s.waiting_crossings_of("alice").unwrap(), 2);
    assert_eq!(
        s.enqueue_review_capped(&cross(1, "alice"), 11, 2).unwrap(),
        Some(a1),
        "a message already waiting is no new place"
    );
    assert!(
        s.enqueue_review_capped(&cross(3, "bob"), 10, 2)
            .unwrap()
            .is_some(),
        "another requester shares the cap"
    );
    s.resolve_review(&ReviewResolution::new(a1, "deny", ""), 20)
        .unwrap();
    assert!(
        s.enqueue_review_capped(&cross(3, "alice"), 21, 2)
            .unwrap()
            .is_some()
    );
}

pub fn a_capped_enqueue_never_hands_a_requester_another_requesters_item<S: Store>(make: fn() -> S) {
    fn cross<'a>(seq: i64, who: &'a str, to: &'a str) -> NewReview<'a> {
        NewReview::new("v", seq, "public", "crossing").crossing(to, who)
    }
    let s = make();
    let bob = s
        .enqueue_review_capped(&cross(1, "bob", "open"), 10, 1)
        .unwrap()
        .unwrap();
    // alice is below the cap: she gets her own item, never bob's id
    let alice = s
        .enqueue_review_capped(&cross(1, "alice", "open"), 11, 1)
        .unwrap()
        .unwrap();
    assert_ne!(alice, bob, "alice was handed bob's item");
    // both are now at the cap: asking for the same message again returns the requester's own item
    assert_eq!(
        s.enqueue_review_capped(&cross(1, "bob", "open"), 12, 1)
            .unwrap(),
        Some(bob)
    );
    assert_eq!(
        s.enqueue_review_capped(&cross(1, "alice", "open"), 12, 1)
            .unwrap(),
        Some(alice)
    );
    // at the cap, a message another member queued is a refusal, not a false "queued"
    assert_eq!(
        s.enqueue_review_capped(&cross(2, "alice", "open"), 13, 1)
            .unwrap(),
        None
    );
    let carol = s
        .enqueue_review_capped(&cross(3, "carol", "open"), 13, 1)
        .unwrap()
        .unwrap();
    assert_eq!(
        s.enqueue_review_capped(&cross(1, "carol", "open"), 14, 1)
            .unwrap(),
        None
    );
    assert_eq!(
        s.enqueue_review_capped(&cross(3, "carol", "open"), 14, 1)
            .unwrap(),
        Some(carol)
    );
    // the same member asking for the same message into another room is a different crossing: refused at the cap
    assert_eq!(
        s.enqueue_review_capped(&cross(1, "bob", "other"), 15, 1)
            .unwrap(),
        None
    );
    // below the cap it is queued as its own item
    let other = s
        .enqueue_review_capped(&cross(1, "bob", "other"), 16, 2)
        .unwrap()
        .unwrap();
    assert_ne!(other, bob);
}
