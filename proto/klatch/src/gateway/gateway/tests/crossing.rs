//! A crossing (#497, DESIGN D4): a member asks that a message in a higher-labelled room be copied into a lower one.
//! The request is queued as a pointer; a verdict (judge or reviewer) releases, redacts or blocks it; the floors are
//! checked again at release.

use super::*;
use crate::domain::Verdict;

fn rev() -> Principal {
    Principal::unattributed("rev")
}

/// `vault` is private with alice cleared to it and one private message; `open` is public. rev is a reviewer cleared to
/// private. Time moves so the breaker stays out of it.
fn world() -> (Gateway<SpyStore>, Rc<std::cell::Cell<i64>>) {
    let (g, t) = timed();
    // the label is set in the policy directly so no canary takes message 1 (the label act plants one, #347)
    g.store
        .policy_set("label:vault", Some("private"), 100)
        .unwrap();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.admin_set_clearance(&admin(), "rev", "private").unwrap();
    g.join(&alice(), "vault").unwrap();
    g.post(&alice(), "vault", "the plan is blue").unwrap();
    g.join(&alice(), "open").unwrap();
    g.join(&bob(), "open").unwrap();
    g.admin_set_reviewer(&admin(), "rev", true).unwrap();
    (g, t)
}

fn bob_sees(g: &Gateway<SpyStore>) -> Vec<String> {
    g.read(&bob(), "open", false)
        .unwrap()
        .into_iter()
        .map(|m| m.body)
        .collect()
}

#[test]
fn a_request_queues_a_pointer_and_moves_nothing() {
    let (g, _) = world();
    let id = g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    let q = g.store.pending_reviews(10).unwrap();
    assert_eq!((q.len(), q[0].id), (1, id));
    assert_eq!(
        (q[0].to_room.as_str(), q[0].requester.as_str()),
        ("open", "alice")
    );
    assert_eq!(q[0].to_label, "public");
    assert!(bob_sees(&g).is_empty(), "nothing crosses before a verdict");
}

#[test]
fn only_a_move_to_a_strictly_lower_label_is_a_crossing() {
    let (g, _) = world();
    g.admin_set_room_label(&admin(), "vault2", "private")
        .unwrap();
    g.join(&alice(), "vault2").unwrap();
    assert!(is_invalid(g.request_crossing(
        &alice(),
        "vault",
        1,
        "vault2"
    )));
    assert!(is_invalid(g.request_crossing(&alice(), "vault", 9, "open")));
    assert!(g.store.pending_reviews(10).unwrap().is_empty());
}

#[test]
fn a_member_who_cannot_read_the_source_cannot_ask() {
    let (g, _) = world();
    let e = g.request_crossing(&bob(), "vault", 1, "open").unwrap_err();
    assert_eq!(rule_of(e), "clearance");
}

#[test]
fn text_that_matches_a_floor_is_refused_at_the_request() {
    let (g, _) = world();
    g.admin_add_scrub_term(&admin(), "codename", "blue")
        .unwrap();
    let e = g
        .request_crossing(&alice(), "vault", 1, "open")
        .unwrap_err();
    assert!(matches!(e, Error::Refused { .. }), "{e}");
    assert!(g.store.pending_reviews(10).unwrap().is_empty());
}

#[test]
fn an_allow_verdict_releases_the_text_into_the_target_room() {
    let (g, _) = world();
    let id = g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    g.review_verdict(&rev(), id, &Verdict::Allow).unwrap();
    assert_eq!(bob_sees(&g), vec!["the plan is blue"]);
    let m = &g.read(&bob(), "open", false).unwrap()[0];
    assert_eq!((m.sender.as_str(), m.label.as_str()), ("alice", "public"));
}

#[test]
fn a_redact_verdict_releases_the_redacted_text_only() {
    let (g, _) = world();
    let id = g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    g.review_verdict(&rev(), id, &Verdict::Redact("the plan is [x]".into()))
        .unwrap();
    assert_eq!(bob_sees(&g), vec!["the plan is [x]"]);
}

#[test]
fn a_deny_verdict_releases_nothing() {
    let (g, _) = world();
    let id = g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    g.review_verdict(&rev(), id, &Verdict::Deny("leaks".into()))
        .unwrap();
    assert!(bob_sees(&g).is_empty());
    assert!(g.release_crossings().unwrap().is_empty());
}

#[test]
fn the_floors_are_checked_again_at_release() {
    let (g, _) = world();
    let id = g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    // the human adds a term after the request and before the verdict: no verdict can let it through
    g.admin_add_scrub_term(&admin(), "codename", "blue")
        .unwrap();
    g.review_verdict(&rev(), id, &Verdict::Allow).unwrap();
    assert!(bob_sees(&g).is_empty());
    let log = g.admin_audit(&admin(), 10).unwrap();
    assert!(
        log.iter()
            .any(|l| l.action == "crossing" && l.detail.contains("blocked"))
    );
    assert!(
        !format!("{log:?}").contains("plan is blue"),
        "the audit holds text"
    );
}

#[test]
fn a_verdict_releases_once_however_often_release_runs() {
    let (g, _) = world();
    let id = g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    g.review_verdict(&rev(), id, &Verdict::Allow).unwrap();
    assert!(g.release_crossings().unwrap().is_empty());
    assert_eq!(bob_sees(&g).len(), 1);
}

#[test]
fn a_judge_verdict_is_applied_by_release_crossings() {
    use crate::domain::{Judge, ReviewItem};
    struct Yes;
    impl Judge for Yes {
        fn judge(&self, _: &ReviewItem) -> Verdict {
            Verdict::Allow
        }
    }
    let (g, _) = world();
    g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    crate::gateway::drain(&g.store, &Yes, 2_000).unwrap();
    assert!(bob_sees(&g).is_empty(), "a verdict alone moves nothing");
    let out = g.release_crossings().unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(bob_sees(&g), vec!["the plan is blue"]);
}

#[test]
fn the_queue_listing_shows_pointers_and_outcomes_never_text() {
    let (g, _) = world();
    let id = g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    let before = g.review_queue(&admin(), 10).unwrap();
    assert_eq!((before.len(), before[0].verdict.as_str()), (1, ""));
    g.review_verdict(&rev(), id, &Verdict::Redact("secret-ish".into()))
        .unwrap();
    let after = g.review_queue(&admin(), 10).unwrap();
    assert_eq!(
        (after[0].verdict.as_str(), after[0].applied.as_str()),
        ("redact", "released")
    );
    assert!(after[0].room == "vault" && after[0].to_room == "open");
}

#[test]
fn a_reviewer_and_a_librarian_cannot_ask_for_a_crossing() {
    let (g, _) = world();
    g.admin_set_librarian(&admin(), "lib", true).unwrap();
    for who in [rev(), Principal::unattributed("lib")] {
        let e = g.request_crossing(&who, "vault", 1, "open").unwrap_err();
        assert!(matches!(e, Error::Refused { .. }), "{e}");
    }
}

#[test]
fn a_crossing_uses_the_messages_own_label_not_the_label_its_room_has_now() {
    let (g, _) = world();
    // vault is relabelled public after message 1 (private) was posted; message 2 is posted under public
    g.admin_set_room_label(&admin(), "vault", "public").unwrap();
    g.post(&alice(), "vault", "an open remark").unwrap();
    let labels: Vec<String> = g
        .tail(&alice(), "vault", (0, 10))
        .unwrap()
        .into_iter()
        .map(|m| m.label)
        .collect();
    assert_eq!(labels, ["private", "public"]);
    // message 1 still carries private, so it may cross to the public room; the room's public label must not decide
    g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    // message 2 is public: nothing is below it in a public room
    assert!(is_invalid(g.request_crossing(&alice(), "vault", 2, "open")));
    assert_eq!(g.store.pending_reviews(10).unwrap().len(), 1);
}

// the sender is told what became of a crossing (#516): a state and a fixed-form reason, never the text, the
// reviewer's reason or the name of a private term

#[test]
fn a_sender_sees_a_waiting_crossing() {
    let (g, _) = world();
    let id = g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    let list = g.my_crossings(&alice(), 10).unwrap();
    assert_eq!((list[0].id, list[0].state.as_str()), (id, "waiting"));
    assert_eq!(
        (list[0].room.as_str(), list[0].to_room.as_str()),
        ("vault", "open")
    );
}

#[test]
fn a_denied_crossing_says_it_was_denied_and_never_the_reviewers_reason() {
    let (g, _) = world();
    let id = g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    g.review_verdict(
        &rev(),
        id,
        &Verdict::Deny("the plan is blue and leaks".into()),
    )
    .unwrap();
    let list = g.my_crossings(&alice(), 10).unwrap();
    assert_eq!(list[0].state, "denied");
    assert!(list[0].why.contains("reviewer"), "{}", list[0].why);
    assert!(!format!("{list:?}").contains("blue"), "{list:?}");
}

#[test]
fn a_blocked_crossing_names_the_category_and_never_the_rule_name_or_term() {
    let (g, _) = world();
    let id = g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    g.admin_add_scrub_term(&admin(), "codename-zed", "blue")
        .unwrap();
    g.review_verdict(&rev(), id, &Verdict::Allow).unwrap();
    let list = g.my_crossings(&alice(), 10).unwrap();
    assert_eq!(list[0].state, "blocked");
    assert!(list[0].why.contains("private-term"), "{}", list[0].why);
    let all = format!("{list:?}");
    assert!(
        !all.contains("codename-zed") && !all.contains("blue"),
        "{all}"
    );
}

#[test]
fn a_released_crossing_says_released() {
    let (g, _) = world();
    let id = g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    g.review_verdict(&rev(), id, &Verdict::Allow).unwrap();
    assert_eq!(g.my_crossings(&alice(), 10).unwrap()[0].state, "released");
}

#[test]
fn a_member_sees_only_their_own_crossings() {
    let (g, _) = world();
    g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    assert!(g.my_crossings(&bob(), 10).unwrap().is_empty());
}
