use super::*;
use crate::domain::Verdict;
use crate::store::NewReview;

fn rev() -> Principal {
    Principal::unattributed("rev")
}

/// A room with one message queued for review and `rev` made the reviewer.
fn with_reviewer() -> (Gateway<SpyStore>, Rc<std::cell::Cell<i64>>) {
    // time moves between refusals so the breaker (five safety refusals in five minutes) stays out of it
    let (g, t) = timed();
    g.join(&alice(), "r").unwrap();
    g.post(&alice(), "r", "candidate text").unwrap();
    g.store
        .enqueue_review(&NewReview::new("r", 1, "public", "crosses"), 1_001)
        .unwrap();
    g.admin_set_reviewer(&admin(), "rev", true).unwrap();
    (g, t)
}

#[test]
fn a_reviewer_reads_the_queue_and_records_a_verdict() {
    let (g, _) = with_reviewer();
    let item = g.review_next(&rev()).unwrap().expect("one item waits");
    assert_eq!(
        (item.room.as_str(), item.text.as_str()),
        ("r", "candidate text")
    );
    g.review_verdict(&rev(), item.id, &Verdict::Deny("leaks".into()))
        .unwrap();
    assert!(g.review_next(&rev()).unwrap().is_none());
    let log = g.admin_audit(&admin(), 5).unwrap();
    assert_eq!(log[0].action, "reviewer");
    assert!(log[0].detail.contains("deny"), "{}", log[0].detail);
    assert!(
        !format!("{log:?}").contains("candidate"),
        "the audit holds text"
    );
}

#[test]
fn a_reviewer_has_no_other_path_to_a_room() {
    let (g, t) = with_reviewer();
    type Attempt<'a> = Box<dyn Fn() -> Option<Error> + 'a>;
    let attempts: Vec<(&str, Attempt)> = vec![
        ("join", Box::new(|| g.join(&rev(), "r").err())),
        ("post", Box::new(|| g.post(&rev(), "r", "hi").err())),
        ("read", Box::new(|| g.read(&rev(), "r", false).err())),
        (
            "advancing read",
            Box::new(|| g.read(&rev(), "r", true).err()),
        ),
        ("who", Box::new(|| g.who(&rev(), "r").err())),
        ("claims", Box::new(|| g.claims(&rev(), "r").err())),
        (
            "claim",
            Box::new(|| g.claim(&rev(), "r", &ClaimSpec::new("p", 10, "")).err()),
        ),
        ("release", Box::new(|| g.release(&rev(), "r", "p").err())),
        ("status", Box::new(|| g.set_status(&rev(), "r", "x").err())),
        ("mute", Box::new(|| g.set_muted(&rev(), "r", true).err())),
        ("ack", Box::new(|| g.ack(&rev(), "r", 1).err())),
        (
            "clear itself",
            Box::new(|| g.admin_set_clearance(&rev(), "rev", "private").err()),
        ),
        (
            "drop its own role",
            Box::new(|| g.admin_set_reviewer(&rev(), "rev", false).err()),
        ),
        (
            "become a librarian",
            Box::new(|| g.admin_set_librarian(&rev(), "rev", true).err()),
        ),
        (
            "close a room",
            Box::new(|| g.admin_close_room(&rev(), "r", true).err()),
        ),
    ];
    for (name, attempt) in attempts {
        t.set(t.get() + 600);
        match attempt() {
            Some(Error::Refused { rule, .. }) => assert_eq!(rule, "reviewer", "{name}"),
            other => panic!("{name} was not refused for a reviewer: {other:?}"),
        }
    }
    let digest = g.digest(&rev()).unwrap();
    assert!(
        digest.rooms.is_empty(),
        "a reviewer is shown no room, only an empty digest"
    );
    let seen = g.read(&bob(), "r", false).unwrap();
    assert_eq!(seen.len(), 1, "nothing was posted");
    assert_eq!(g.clearance_of("rev").unwrap(), "public");
}

#[test]
fn only_a_reviewer_may_use_the_queue() {
    let (g, _) = with_reviewer();
    assert!(is_refused(g.review_next(&bob())));
    assert!(is_refused(g.review_verdict(&bob(), 1, &Verdict::Allow)));
    assert_eq!(
        g.store.pending_reviews(10).unwrap().len(),
        1,
        "still queued"
    );
}

#[test]
fn a_verdict_is_one_of_three_and_is_final() {
    let (g, _) = with_reviewer();
    let id = g.review_next(&rev()).unwrap().unwrap().id;
    assert!(is_invalid(g.review_verdict(&rev(), id, &Verdict::Defer)));
    assert!(is_invalid(g.review_verdict(
        &rev(),
        id + 50,
        &Verdict::Allow
    )));
    g.review_verdict(&rev(), id, &Verdict::Deny("no".into()))
        .unwrap();
    assert!(
        is_invalid(g.review_verdict(&rev(), id, &Verdict::Allow)),
        "a deny cannot be overridden"
    );
}

#[test]
fn a_reviewer_is_held_to_clearance() {
    let (g, _) = with_reviewer();
    g.admin_set_room_label(&admin(), "r", "private").unwrap();
    assert!(
        g.review_next(&rev()).unwrap().is_none(),
        "above its clearance"
    );
    let id = g.store.pending_reviews(1).unwrap()[0].id;
    assert!(is_refused(g.review_verdict(&rev(), id, &Verdict::Allow)));
    g.admin_set_clearance(&admin(), "rev", "private").unwrap();
    assert!(g.review_next(&rev()).unwrap().is_some());
}

#[test]
fn a_member_holds_one_role_and_it_can_be_taken_away() {
    let (g, _) = with_reviewer();
    g.admin_set_librarian(&admin(), "rev", true).unwrap();
    assert!(is_refused(g.review_next(&rev())), "now a librarian");
    g.admin_set_reviewer(&admin(), "rev", true).unwrap();
    g.admin_set_librarian(&admin(), "rev", false).unwrap();
    assert!(
        g.review_next(&rev()).is_ok(),
        "dropping another role keeps this one"
    );
    g.admin_set_reviewer(&admin(), "rev", false).unwrap();
    assert!(g.post(&rev(), "r", "ordinary again").is_ok());
    assert!(is_invalid(g.admin_set_reviewer(&admin(), "bad name", true)));
}
