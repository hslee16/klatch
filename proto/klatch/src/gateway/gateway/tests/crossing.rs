//! A crossing (#497, DESIGN D4): a member asks that a message in a higher-labelled room be copied into a lower one.
//! The request is queued as a pointer; a verdict (judge or reviewer) releases, redacts or blocks it; the floors are
//! checked again at release.

use super::super::crossing::DEFAULT_CROSSING_CAP;
use super::*;
use crate::domain::Verdict;

fn judge() -> Principal {
    Principal::unattributed("judge")
}

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
    assert!(g.release_crossings(&judge()).unwrap().is_empty());
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
    assert!(g.release_crossings(&judge()).unwrap().is_empty());
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
    let out = g.release_crossings(&judge()).unwrap();
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

// the target room must be joined (#679): a crossing never makes a member of anyone

#[test]
fn a_crossing_into_a_room_the_asker_has_not_joined_is_refused_with_a_clear_message() {
    let (g, _) = world();
    g.admin_set_room_label(&admin(), "lobby", "public").unwrap();
    let e = g
        .request_crossing(&alice(), "vault", 1, "lobby")
        .unwrap_err();
    assert!(matches!(e, Error::Invalid(_)), "{e}");
    assert!(
        e.to_string().contains("join lobby"),
        "the message names the fix: {e}"
    );
    assert!(g.store.pending_reviews(10).unwrap().is_empty());
}

// a released message is marked kind crossed with a pointer at the source, never the source text (#679)

#[test]
fn a_released_message_has_kind_crossed_and_a_reference_to_the_source() {
    let (g, _) = world();
    let id = g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    g.review_verdict(&rev(), id, &Verdict::Redact("the plan is [x]".into()))
        .unwrap();
    let m = &g.read(&bob(), "open", false).unwrap()[0];
    assert_eq!(m.kind.as_deref(), Some("crossed"));
    assert_eq!(m.payload.as_deref(), Some("vault#1"));
    assert_eq!(m.body, "the plan is [x]");
}

// the requester is told once in its digest, nothing is posted in either room (#679)

fn told(g: &Gateway<SpyStore>) -> Vec<String> {
    g.digest(&alice())
        .unwrap()
        .rooms
        .iter()
        .flat_map(|r| r.crossings.iter().map(|c| c.state.clone()))
        .collect()
}

#[test]
fn the_requester_is_told_waiting_once_then_the_outcome_once() {
    let (g, _) = world();
    let id = g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    assert_eq!(told(&g), ["waiting"]);
    assert!(told(&g).is_empty(), "waiting is said once");
    g.review_verdict(&rev(), id, &Verdict::Allow).unwrap();
    assert_eq!(told(&g), ["released"]);
    assert!(told(&g).is_empty(), "the outcome is said once");
}

#[test]
fn a_refusal_is_told_with_the_rule_never_the_text_and_posts_nothing() {
    let (g, _) = world();
    let id = g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    g.review_verdict(&rev(), id, &Verdict::Deny("blue leaks".into()))
        .unwrap();
    let d = g.digest(&alice()).unwrap();
    let all: Vec<_> = d.rooms.iter().flat_map(|r| &r.crossings).collect();
    assert_eq!((all.len(), all[0].state.as_str()), (1, "denied"));
    assert!(!format!("{all:?}").contains("blue"), "{all:?}");
    assert!(bob_sees(&g).is_empty());
    assert_eq!(g.tail(&alice(), "vault", (0, 10)).unwrap().len(), 1);
}

#[test]
fn a_blocked_release_is_told_to_the_requester() {
    let (g, _) = world();
    let id = g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    g.admin_add_scrub_term(&admin(), "codename-zed", "blue")
        .unwrap();
    g.review_verdict(&rev(), id, &Verdict::Allow).unwrap();
    assert_eq!(told(&g), ["blocked"]);
}

#[test]
fn another_member_is_not_told_about_a_crossing() {
    let (g, _) = world();
    g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    let d = g.digest(&bob()).unwrap();
    assert!(d.rooms.iter().all(|r| r.crossings.is_empty()));
}

#[test]
fn a_release_names_who_released_it_in_the_audit() {
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
    g.release_crossings(&judge()).unwrap();
    let log = g.admin_audit(&admin(), 10).unwrap();
    assert!(
        log.iter()
            .any(|l| l.action == "crossing" && l.detail.contains("applied by judge")),
        "{log:?}"
    );
}

#[test]
fn a_reviewers_own_verdict_names_the_reviewer_as_the_releaser() {
    let (g, _) = world();
    let id = g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    g.review_verdict(&rev(), id, &Verdict::Allow).unwrap();
    let log = g.admin_audit(&admin(), 10).unwrap();
    assert!(
        log.iter()
            .any(|l| l.action == "crossing" && l.detail.contains("applied by rev")),
        "{log:?}"
    );
}

#[test]
fn a_principal_without_the_role_cannot_release() {
    let (g, _) = world();
    g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    // a reviewer may only review, so it may not drive the drain
    assert_eq!(
        rule_of(g.release_crossings(&rev()).unwrap_err()),
        "reviewer"
    );
    // an admin agent has no release act on its list
    g.admin_set_admin_agent(&admin(), "boss", true).unwrap();
    let boss = Principal::unattributed("boss");
    assert_eq!(
        rule_of(g.release_crossings(&boss).unwrap_err()),
        "admin-scope"
    );
    // on a members' gateway a plain member is refused too
    let (m, _) = timed();
    let m = m.for_members();
    assert_eq!(
        rule_of(m.release_crossings(&alice()).unwrap_err()),
        "admin-scope"
    );
}

/// Post `n` more messages in the vault, so crossings have distinct sources.
fn more_vault_messages(g: &Gateway<SpyStore>, n: usize) {
    for i in 0..n {
        g.post(&alice(), "vault", &format!("another fact {i}"))
            .unwrap();
    }
}

#[test]
fn a_requester_cannot_have_more_than_the_cap_of_crossings_waiting() {
    let (g, _) = world();
    let cap = DEFAULT_CROSSING_CAP;
    more_vault_messages(&g, cap);
    for seq in 1..=cap as i64 {
        g.request_crossing(&alice(), "vault", seq, "open").unwrap();
    }
    let e = g
        .request_crossing(&alice(), "vault", cap as i64 + 1, "open")
        .unwrap_err();
    let shown = format!("{e}");
    assert_eq!(rule_of(e), "crossing-cap");
    assert!(shown.contains(&cap.to_string()), "{shown}");
    assert!(
        !shown.contains("another fact") && !shown.contains("the plan"),
        "the refusal echoed message text: {shown}"
    );
    assert_eq!(g.store.pending_reviews(1000).unwrap().len(), cap);
}

#[test]
fn the_cap_is_per_requester_and_frees_up_when_an_item_is_decided() {
    let (g, _) = world();
    g.admin_set_clearance(&admin(), "bob", "private").unwrap();
    g.join(&bob(), "vault").unwrap();
    g.admin_set_crossing_cap(&admin(), Some(2)).unwrap();
    more_vault_messages(&g, 3);
    g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    let second = g.request_crossing(&alice(), "vault", 2, "open").unwrap();
    assert!(g.request_crossing(&alice(), "vault", 3, "open").is_err());
    // another requester is not held up by alice's queue
    g.request_crossing(&bob(), "vault", 3, "open").unwrap();
    // a decided item no longer waits
    g.store
        .resolve_review(&crate::store::ReviewResolution::new(second, "deny", ""), 5)
        .unwrap();
    g.request_crossing(&alice(), "vault", 3, "open").unwrap();
}

#[test]
fn the_human_sets_the_cap_and_off_restores_the_default() {
    let (g, _) = world();
    assert_eq!(g.crossing_cap().unwrap(), DEFAULT_CROSSING_CAP);
    g.admin_set_crossing_cap(&admin(), Some(7)).unwrap();
    assert_eq!(g.crossing_cap().unwrap(), 7);
    assert_eq!(
        g.store.policy_get("crossing:cap").unwrap().as_deref(),
        Some("7")
    );
    assert!(g.admin_set_crossing_cap(&admin(), Some(0)).is_err());
    g.admin_set_crossing_cap(&admin(), None).unwrap();
    assert_eq!(g.crossing_cap().unwrap(), DEFAULT_CROSSING_CAP);
    // an unreadable stored value falls back to the default, so the cap stays in force
    g.store.policy_set("crossing:cap", Some("junk"), 1).unwrap();
    assert_eq!(g.crossing_cap().unwrap(), DEFAULT_CROSSING_CAP);
    // zero would refuse every crossing and a huge value would lift the bound: both are ignored
    for bad in ["0", "99999999"] {
        g.store.policy_set("crossing:cap", Some(bad), 1).unwrap();
        assert_eq!(g.crossing_cap().unwrap(), DEFAULT_CROSSING_CAP, "{bad}");
    }
}

#[test]
fn a_refusal_at_the_cap_is_recorded_with_its_rule_and_no_message_text() {
    let (g, _) = world();
    g.admin_set_crossing_cap(&admin(), Some(1)).unwrap();
    more_vault_messages(&g, 1);
    g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    assert!(g.request_crossing(&alice(), "vault", 2, "open").is_err());
    let rows: Vec<_> = g
        .my_refusals(&alice(), 50)
        .unwrap()
        .into_iter()
        .filter(|r| r.rule == "crossing-cap")
        .collect();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(
        (rows[0].member.as_str(), rows[0].operation.as_str()),
        ("alice", "crossing")
    );
    let shown = format!("{rows:?}");
    assert!(
        !shown.contains("the plan") && !shown.contains("another fact"),
        "{shown}"
    );
}

#[test]
fn a_flood_of_refusals_at_the_cap_keeps_a_bounded_number_of_rows_and_refuses_every_time() {
    let (g, t) = world();
    g.admin_set_crossing_cap(&admin(), Some(1)).unwrap();
    more_vault_messages(&g, 1);
    g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    let rows = |g: &Gateway<SpyStore>| {
        g.my_refusals(&alice(), 1000)
            .unwrap()
            .iter()
            .filter(|r| r.rule == "crossing-cap")
            .count()
    };
    for _ in 0..500 {
        let e = g
            .request_crossing(&alice(), "vault", 2, "open")
            .unwrap_err();
        assert_eq!(rule_of(e), "crossing-cap");
    }
    assert_eq!(rows(&g), 1, "500 refusals in one minute are one row");
    t.set(t.get() + 61);
    assert!(g.request_crossing(&alice(), "vault", 2, "open").is_err());
    assert_eq!(rows(&g), 2, "a later minute is told again");
}

#[test]
fn a_message_already_waiting_is_returned_at_the_cap_and_adds_nothing() {
    let (g, _) = world();
    g.admin_set_crossing_cap(&admin(), Some(1)).unwrap();
    let id = g.request_crossing(&alice(), "vault", 1, "open").unwrap();
    // the queue dedupes the same message, so asking again is no new place
    assert_eq!(
        g.request_crossing(&alice(), "vault", 1, "open").unwrap(),
        id
    );
}
