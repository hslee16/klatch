//! Retention and thresholds that fit hours and days (#306, DESIGN section 32 item 44).

use super::*;

const DAY: i64 = 86_400;

#[test]
fn a_member_that_holds_a_live_claim_is_not_pruned_and_keeps_its_cursor() {
    let s = store();
    s.join("r", "alice", 10).unwrap();
    s.join("r", "bob", 10).unwrap();
    s.post(Seat::new("r", "carol"), "before", 11).unwrap();
    s.read(Seat::new("r", "alice"), true, 12).unwrap();
    s.note_session("sess-alice", "alice", 10).unwrap();
    s.claim(
        &ClaimRequest::new(("r", "alice"), "f.rs", 10 + 3 * DAY, ""),
        10,
    )
    .unwrap();
    let removed = s.prune_members(DAY, 2 * DAY).unwrap();
    assert_eq!(removed, 2, "bob and carol go, alice holds a claim");
    let names: Vec<_> = s
        .members("r")
        .unwrap()
        .into_iter()
        .map(|m| m.name)
        .collect();
    assert_eq!(names, ["alice"]);
    assert_eq!(
        s.member_of_session("sess-alice").unwrap().as_deref(),
        Some("alice")
    );
    s.post(Seat::new("r", "carol"), "after two days", 2 * DAY)
        .unwrap();
    let unread = s.read(Seat::new("r", "alice"), true, 2 * DAY + 1).unwrap();
    assert_eq!(
        unread.len(),
        1,
        "a resumed session does not re-read the room"
    );
}

#[test]
fn an_expired_claim_no_longer_protects_its_holder() {
    let s = store();
    s.join("r", "alice", 10).unwrap();
    s.claim(&ClaimRequest::new(("r", "alice"), "f.rs", 100, ""), 10)
        .unwrap();
    assert_eq!(s.prune_members(DAY, 2 * DAY).unwrap(), 1);
    assert!(s.members("r").unwrap().is_empty());
}

#[test]
fn a_claim_in_one_room_protects_the_member_in_every_room() {
    let s = store();
    s.join("r", "alice", 10).unwrap();
    s.join("r2", "alice", 10).unwrap();
    s.claim(
        &ClaimRequest::new(("r2", "alice"), "f.rs", 10 + 3 * DAY, ""),
        10,
    )
    .unwrap();
    assert_eq!(s.prune_members(DAY, 2 * DAY).unwrap(), 0);
    assert_eq!(s.members("r").unwrap().len(), 1);
}

#[test]
fn a_claimed_ask_is_never_stale_however_old_and_a_deadline_moves_the_report() {
    let s = store();
    let day = 20_777 * DAY;
    let t0 = day - 5 * DAY;
    let long = s
        .post(Seat::new("r", "alice"), "seeking: slow", t0)
        .unwrap()
        .seq;
    s.claim(
        &ClaimRequest::new(("r", "bob"), &format!("ask:{long}"), day + 3 * DAY, ""),
        t0 + 4 * DAY,
    )
    .unwrap();
    let dated = s
        .post(
            Seat::new("r", "alice"),
            "seeking: by friday (by 2026-11-20)",
            t0,
        )
        .unwrap()
        .seq;
    let at = |now| s.stale_asks(Seat::new("r", "alice"), now, 60).unwrap();
    assert!(
        at(t0 + 2 * DAY).is_empty(),
        "before the deadline the dated ask is not stale"
    );
    let late = at(day + DAY);
    let seqs: Vec<_> = late.iter().map(|a| a.seq).collect();
    assert_eq!(seqs, [dated], "after the deadline only the unclaimed one");
    assert!(at(day + 2 * DAY).is_empty(), "reported once");
}
