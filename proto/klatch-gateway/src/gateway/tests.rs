use super::*;
use crate::test_support::{SpyGuard, SpyStore};
use crate::{AllowAll, FixedClock, Operation};
use klatch_core::Error;
use std::rc::Rc;

fn alice() -> Principal {
    Principal::unattributed("alice")
}
fn bob() -> Principal {
    Principal::unattributed("bob")
}
fn open() -> Gateway<SpyStore> {
    Gateway::new(SpyStore::new(), FixedClock(100), AllowAll)
}

#[test]
fn post_then_read_round_trips_through_the_gateway() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    let p = g.post(&bob(), "r", "hello").unwrap();
    let got = g.read(&alice(), "r", true).unwrap();
    assert_eq!(
        (got[0].sender.as_str(), got[0].body.as_str(), got[0].seq),
        ("bob", "hello", p.seq)
    );
    assert!(g.read(&alice(), "r", true).unwrap().is_empty());
}

#[test]
fn the_injected_clock_is_the_only_source_of_time() {
    let g = open();
    g.claim(&alice(), "r", "src", 10, "").unwrap();
    assert_eq!(g.claims(&alice(), "r").unwrap()[0].expires, 110);
    assert_eq!(g.now(), 100);
}

#[test]
fn every_operation_validates_the_room_name_before_anything_else() {
    let g = open();
    for room in ["bad room", "a\nb", "", "x/y"] {
        let who = alice();
        assert!(
            matches!(g.join(&who, room), Err(Error::Invalid(_))),
            "join {room:?}"
        );
        assert!(
            matches!(g.post(&who, room, "x"), Err(Error::Invalid(_))),
            "post {room:?}"
        );
        assert!(
            matches!(g.read(&who, room, true), Err(Error::Invalid(_))),
            "read {room:?}"
        );
        assert!(
            matches!(g.who(&who, room), Err(Error::Invalid(_))),
            "who {room:?}"
        );
        assert!(
            matches!(g.claim(&who, room, "p", 10, ""), Err(Error::Invalid(_))),
            "claim {room:?}"
        );
        assert!(
            matches!(g.release(&who, room, "p"), Err(Error::Invalid(_))),
            "release {room:?}"
        );
        assert!(
            matches!(g.claims(&who, room), Err(Error::Invalid(_))),
            "claims {room:?}"
        );
    }
    assert_eq!(
        g.store.calls.get(),
        0,
        "an invalid request must not reach the store"
    );
}

#[test]
fn an_invalid_member_name_is_refused_everywhere() {
    let g = open();
    let bad = Principal::unattributed("bad name\nforged");
    assert!(matches!(g.join(&bad, "r"), Err(Error::Invalid(_))));
    assert!(matches!(g.post(&bad, "r", "x"), Err(Error::Invalid(_))));
    assert!(matches!(g.digest(&bad), Err(Error::Invalid(_))));
    assert_eq!(g.store.calls.get(), 0);
}

#[test]
fn claim_paths_and_reasons_are_checked() {
    let g = open();
    let who = alice();
    for path in ["", "a\nb", "a\rb", "a\u{0}b", &"x".repeat(1025)] {
        assert!(
            matches!(g.claim(&who, "r", path, 10, ""), Err(Error::Invalid(_))),
            "path {path:?}"
        );
    }
    assert!(matches!(
        g.claim(&who, "r", "src", 10, "bad\nreason"),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        g.claim(&who, "r", "src", 10, &"y".repeat(501)),
        Err(Error::Invalid(_))
    ));
    g.claim(&who, "r", "src/a b.rs", 10, "a fine reason")
        .unwrap();
    // Only the valid claim reached the store: it looks up the member's scope, then claims.
    assert_eq!(g.store.calls.get(), 2);
}

#[test]
fn a_claim_needs_a_positive_ttl() {
    let g = open();
    assert!(matches!(
        g.claim(&alice(), "r", "src", 0, ""),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        g.claim(&alice(), "r", "src", -5, ""),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn the_guard_runs_once_per_operation_in_order_before_the_store() {
    let guard = Rc::new(SpyGuard::allowing());
    let g = Gateway::new(SpyStore::new(), FixedClock(1), guard.clone());
    let who = alice();
    g.join(&who, "r").unwrap();
    g.post(&who, "r", "hi").unwrap();
    g.read(&who, "r", false).unwrap();
    g.who(&who, "r").unwrap();
    g.claim(&who, "r", "src", 10, "").unwrap();
    g.claims(&who, "r").unwrap();
    g.release(&who, "r", "src").unwrap();
    let kinds: Vec<&str> = guard
        .seen
        .borrow()
        .iter()
        .map(|o| match o {
            Operation::Join { .. } => "join",
            Operation::Post { .. } => "post",
            Operation::Read { .. } => "read",
            Operation::Who { .. } => "who",
            Operation::Claim { .. } => "claim",
            Operation::Claims { .. } => "claims",
            Operation::Release { .. } => "release",
            Operation::Subscribe { .. } => "subscribe",
            Operation::Status { .. } => "status",
            Operation::Ack { .. } => "ack",
            Operation::Admin { .. } => "admin",
        })
        .collect();
    assert_eq!(
        kinds,
        vec!["join", "post", "read", "who", "claim", "claims", "release"]
    );
}

struct TestClock(Rc<std::cell::Cell<i64>>);
impl Clock for TestClock {
    fn now(&self) -> i64 {
        self.0.get()
    }
}

fn timed() -> (Gateway<SpyStore>, Rc<std::cell::Cell<i64>>) {
    let t = Rc::new(std::cell::Cell::new(1_000));
    (
        Gateway::new(SpyStore::new(), TestClock(t.clone()), AllowAll),
        t,
    )
}

#[test]
fn a_post_over_the_size_cap_is_refused_with_the_reason_and_stores_nothing() {
    let g = Gateway::new(SpyStore::new(), FixedClock(1), AllowAll);
    let at_cap = "a".repeat(klatch_core::budget::MAX_POST_BYTES);
    g.post(&alice(), "r", &at_cap).unwrap();
    let err = g
        .post(&alice(), "r", &format!("{at_cap}a"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("too large") && err.contains("32768"), "{err}");
    assert_eq!(g.store.head("r").unwrap(), 1);
}

#[test]
fn an_identical_repeat_within_a_minute_is_refused_naming_the_original() {
    let (g, clock) = timed();
    let first = g.post(&alice(), "r", "closed #3").unwrap();
    clock.set(1_010);
    let err = g.post(&alice(), "r", "closed #3").unwrap_err().to_string();
    assert!(
        err.contains(&format!("same as your message #{}", first.seq)) && err.contains("10s ago"),
        "{err}"
    );
    g.post(&bob(), "r", "closed #3").unwrap();
    g.post(&alice(), "other", "closed #3").unwrap();
    g.post(&alice(), "r", "closed #4").unwrap();
    clock.set(1_061);
    g.post(&alice(), "r", "closed #3").unwrap();
}

#[test]
fn a_burst_of_posts_is_refused_with_how_long_to_wait_and_clears_with_time() {
    let (g, clock) = timed();
    for i in 0..klatch_core::budget::MAX_POSTS_PER_WINDOW {
        g.post(&alice(), "r", &format!("m{i}")).unwrap();
    }
    let err = g
        .post(&alice(), "r", "one too many")
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("at most 100 posts in 60 seconds") && err.contains("wait"),
        "{err}"
    );
    g.post(&bob(), "r", "someone else is not limited").unwrap();
    clock.set(1_061);
    g.post(&alice(), "r", "after the window").unwrap();
}

fn admin() -> Principal {
    Principal::unattributed("admin")
}

fn rule_of(e: klatch_core::Error) -> String {
    match e {
        Error::Refused { rule, .. } => rule,
        other => panic!("not a refusal: {other}"),
    }
}

#[test]
fn a_paused_gateway_refuses_every_room_operation_and_resumes() {
    let g = Gateway::new(SpyStore::new(), FixedClock(1), AllowAll);
    g.join(&alice(), "r").unwrap();
    g.admin_pause(&admin(), true).unwrap();
    assert_eq!(
        rule_of(g.post(&alice(), "r", "x").unwrap_err()),
        "admin-pause"
    );
    assert_eq!(rule_of(g.join(&bob(), "r").unwrap_err()), "admin-pause");
    assert_eq!(
        rule_of(g.claim(&alice(), "r", "a", 60, "").unwrap_err()),
        "admin-pause"
    );
    assert_eq!(
        rule_of(g.read(&alice(), "r", false).unwrap_err()),
        "admin-pause"
    );
    let d = g.digest(&alice()).unwrap();
    assert!(d.rooms.is_empty() && d.refused.len() == 1, "{d:?}");
    assert!(d.refused[0].reason.contains("paused by the admin"));
    g.admin_pause(&admin(), false).unwrap();
    g.post(&alice(), "r", "x").unwrap();
}

#[test]
fn a_closed_room_stays_readable_but_takes_no_posts_claims_or_statuses() {
    let g = Gateway::new(SpyStore::new(), FixedClock(1), AllowAll);
    g.post(&alice(), "r", "history").unwrap();
    g.claim(&alice(), "r", "a", 60, "").unwrap();
    g.admin_close_room(&admin(), "r", true).unwrap();
    assert_eq!(
        rule_of(g.post(&bob(), "r", "x").unwrap_err()),
        "admin-close"
    );
    assert_eq!(
        rule_of(g.claim(&bob(), "r", "b", 60, "").unwrap_err()),
        "admin-close"
    );
    assert_eq!(
        rule_of(g.set_status(&bob(), "r", "done").unwrap_err()),
        "admin-close"
    );
    assert_eq!(g.read(&bob(), "r", false).unwrap()[0].body, "history");
    assert_eq!(g.claims(&bob(), "r").unwrap().len(), 1);
    g.release(&alice(), "r", "a").unwrap();
    g.post(&bob(), "other", "elsewhere is fine").unwrap();
    g.admin_close_room(&admin(), "r", false).unwrap();
    g.post(&bob(), "r", "open again").unwrap();
}

#[test]
fn a_removed_member_is_refused_in_that_room_only_until_allowed() {
    let g = Gateway::new(SpyStore::new(), FixedClock(1), AllowAll);
    g.join(&bob(), "r").unwrap();
    g.join(&bob(), "side").unwrap();
    g.claim(&bob(), "r", "a", 60, "").unwrap();
    g.admin_remove_member(&admin(), "r", "bob").unwrap();
    assert_eq!(
        rule_of(g.post(&bob(), "r", "x").unwrap_err()),
        "admin-block"
    );
    assert_eq!(rule_of(g.join(&bob(), "r").unwrap_err()), "admin-block");
    assert!(
        g.claims(&alice(), "r").unwrap().is_empty(),
        "its claims were released"
    );
    g.post(&bob(), "side", "still fine in another room")
        .unwrap();
    g.post(&alice(), "r", "others are fine").unwrap();
    g.admin_allow_member(&admin(), "r", "bob").unwrap();
    g.post(&bob(), "r", "back").unwrap();
    assert!(matches!(
        g.admin_allow_member(&admin(), "r", "bob"),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn removing_a_member_from_every_room_blocks_it_everywhere() {
    let g = Gateway::new(SpyStore::new(), FixedClock(1), AllowAll);
    g.join(&bob(), "r").unwrap();
    g.join(&bob(), "side").unwrap();
    g.admin_remove_member(&admin(), "*", "bob").unwrap();
    for room in ["r", "side", "never-joined"] {
        assert_eq!(
            rule_of(g.post(&bob(), room, "x").unwrap_err()),
            "admin-block"
        );
    }
    g.admin_allow_member(&admin(), "*", "bob").unwrap();
    g.post(&bob(), "side", "x").unwrap();
}

#[test]
fn every_administrative_act_is_audited_newest_first_and_a_failed_one_is_not() {
    let g = Gateway::new(SpyStore::new(), FixedClock(1), AllowAll);
    g.admin_close_room(&admin(), "r", true).unwrap();
    g.admin_pause(&admin(), true).unwrap();
    let _ = g.admin_allow_member(&admin(), "r", "nobody");
    let log = g.admin_audit(&admin(), 10).unwrap();
    let acts: Vec<(&str, &str)> = log
        .iter()
        .map(|a| (a.action.as_str(), a.target.as_str()))
        .collect();
    assert_eq!(acts, vec![("pause", "*"), ("close", "r")]);
}

#[test]
fn the_guard_sees_an_admin_act_before_it_happens() {
    let guard = Rc::new(SpyGuard::denying(&["admin"]));
    let g = Gateway::new(SpyStore::new(), FixedClock(1), guard.clone());
    let _ = g.admin_pause(&admin(), true);
    assert!(matches!(
        guard.seen.borrow()[0],
        Operation::Admin { ref action, .. } if action == "pause"
    ));
}

#[test]
fn the_guard_sees_the_post_body_so_a_label_check_can_judge_it() {
    let guard = Rc::new(SpyGuard::allowing());
    let g = Gateway::new(SpyStore::new(), FixedClock(1), guard.clone());
    g.post(&alice(), "r", "the body").unwrap();
    assert_eq!(
        guard.seen.borrow()[0],
        Operation::Post {
            room: "r".into(),
            body: "the body".into()
        }
    );
}

#[test]
fn a_denied_operation_never_reaches_the_store_and_returns_the_guards_error() {
    let g = Gateway::new(
        SpyStore::new(),
        FixedClock(1),
        Rc::new(SpyGuard::denying(&["locked"])),
    );
    let who = alice();
    let err = g.post(&who, "locked", "x").unwrap_err();
    assert!(
        matches!(err, Error::Refused { ref rule, .. } if rule == "test.deny_room"),
        "{err}"
    );
    assert!(g.read(&who, "locked", true).is_err());
    assert!(g.claim(&who, "locked", "p", 10, "").is_err());
    assert_eq!(g.store.calls.get(), 0);
    g.post(&who, "open", "x").unwrap();
}

#[test]
fn store_failures_surface_as_errors_never_as_success() {
    let g = open();
    g.store.fail.set(true);
    let who = alice();
    assert!(matches!(g.post(&who, "r", "x"), Err(Error::Backend(_))));
    assert!(matches!(g.read(&who, "r", true), Err(Error::Backend(_))));
    assert!(matches!(
        g.claim(&who, "r", "p", 10, ""),
        Err(Error::Backend(_))
    ));
    assert!(matches!(g.claims(&who, "r"), Err(Error::Backend(_))));
    assert!(matches!(g.digest(&who), Err(Error::Backend(_))));
}

#[test]
fn tail_head_and_ack_validate_before_the_store_and_go_through_the_guard() {
    let guard = Rc::new(SpyGuard::allowing());
    let g = Gateway::new(SpyStore::new(), FixedClock(1), guard.clone());
    let who = alice();
    for room in ["bad room", "a\nb", ""] {
        assert!(matches!(g.tail(&who, room, 0, 10), Err(Error::Invalid(_))));
        assert!(matches!(g.head(&who, room), Err(Error::Invalid(_))));
        assert!(matches!(g.ack(&who, room, 1), Err(Error::Invalid(_))));
    }
    assert!(matches!(g.tail(&who, "r", -1, 10), Err(Error::Invalid(_))));
    assert!(matches!(g.tail(&who, "r", 0, 0), Err(Error::Invalid(_))));
    assert!(matches!(g.ack(&who, "r", -1), Err(Error::Invalid(_))));
    assert_eq!(g.store.calls.get(), 0);
    assert!(
        guard.seen.borrow().is_empty(),
        "invalid requests never reach the guard"
    );
    g.tail(&who, "r", 0, 10).unwrap();
    g.head(&who, "r").unwrap();
    g.ack(&who, "r", 0).unwrap();
    assert_eq!(
        *guard.seen.borrow(),
        vec![
            Operation::Subscribe { room: "r".into() },
            Operation::Subscribe { room: "r".into() },
            Operation::Ack { room: "r".into() },
        ]
    );
}

#[test]
fn tail_includes_everyones_messages_and_head_reports_the_top_seq() {
    let g = open();
    g.post(&alice(), "r", "one").unwrap();
    g.post(&bob(), "r", "two").unwrap();
    assert_eq!(g.tail(&alice(), "r", 0, 10).unwrap().len(), 2);
    assert_eq!(g.head(&alice(), "r").unwrap(), 2);
}

#[test]
fn an_ack_moves_the_read_cursor_so_the_hook_path_does_not_redeliver() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    for i in 0..3 {
        g.post(&bob(), "r", &format!("m{i}")).unwrap();
    }
    g.ack(&alice(), "r", 2).unwrap();
    let unread = g.read(&alice(), "r", false).unwrap();
    assert_eq!(unread.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![3]);
}

#[test]
fn a_denied_tail_never_reaches_the_store() {
    let g = Gateway::new(
        SpyStore::new(),
        FixedClock(1),
        Rc::new(SpyGuard::denying(&["locked"])),
    );
    assert!(matches!(
        g.tail(&alice(), "locked", 0, 10),
        Err(Error::Refused { .. })
    ));
    assert!(matches!(
        g.ack(&alice(), "locked", 1),
        Err(Error::Refused { .. })
    ));
    assert_eq!(g.store.calls.get(), 0);
}

#[test]
fn a_verified_principal_posts_a_verified_message_and_an_unattributed_one_stays_unattributed() {
    let g = open();
    g.post(&Principal::verified("carol"), "r", "proven")
        .unwrap();
    g.post(&Principal::unattributed("eve"), "r", "unproven")
        .unwrap();
    let got = g.tail(&alice(), "r", 0, 10).unwrap();
    assert_eq!(got[0].attribution, klatch_core::Attribution::Verified);
    assert_eq!(got[1].attribution, klatch_core::Attribution::Unattributed);
}

#[test]
fn messages_are_posted_under_the_default_label() {
    let g = open();
    g.post(&alice(), "r", "x").unwrap();
    assert_eq!(
        g.tail(&alice(), "r", 0, 10).unwrap()[0].label,
        DEFAULT_LABEL
    );
    assert_eq!(DEFAULT_LABEL, "public");
}

#[test]
fn an_empty_or_whitespace_only_body_is_refused_before_the_guard_and_the_store() {
    let guard = Rc::new(SpyGuard::allowing());
    let g = Gateway::new(SpyStore::new(), FixedClock(1), guard.clone());
    for body in ["", " ", "   \t ", "\n", " \r\n "] {
        assert!(
            matches!(g.post(&alice(), "r", body), Err(Error::Invalid(_))),
            "body {body:?}"
        );
    }
    assert_eq!(g.store.calls.get(), 0);
    assert!(guard.seen.borrow().is_empty());
    g.post(&alice(), "r", " x ").unwrap();
}

#[test]
fn a_digest_collects_unread_and_others_claims_and_advances_the_cursor() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.post(&bob(), "r", "ping").unwrap();
    g.claim(&bob(), "r", "src", 50, "").unwrap();
    g.claim(&alice(), "r", "docs", 50, "").unwrap();
    let d = g.digest(&alice()).unwrap();
    assert_eq!(d.now, 100);
    assert_eq!(d.rooms.len(), 1);
    assert_eq!(d.rooms[0].messages[0].body, "ping");
    assert_eq!(
        d.rooms[0]
            .claims
            .iter()
            .map(|c| c.owner.as_str())
            .collect::<Vec<_>>(),
        vec!["bob"]
    );
    assert!(g.digest(&alice()).unwrap().rooms[0].messages.is_empty());
}

#[test]
fn a_message_a_digest_showed_can_still_be_read_once() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.post(&bob(), "r", "ask").unwrap();
    assert_eq!(g.digest(&alice()).unwrap().rooms[0].messages.len(), 1);
    assert!(g.digest(&alice()).unwrap().rooms[0].messages.is_empty());
    assert_eq!(g.read(&alice(), "r", true).unwrap().len(), 1);
    assert!(g.read(&alice(), "r", true).unwrap().is_empty());
}

#[test]
fn a_message_a_wait_has_seen_is_still_unread_and_the_wait_does_not_wake_again() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.post(&bob(), "r", "ask").unwrap();
    assert_eq!(g.wait_poll(&alice(), "r").unwrap().len(), 1);
    g.wait_seen(&alice(), "r", 1).unwrap();
    assert!(g.wait_poll(&alice(), "r").unwrap().is_empty());
    assert_eq!(g.read(&alice(), "r", false).unwrap().len(), 1);
}

#[test]
fn open_asks_go_through_the_gateway() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.post(&bob(), "r", "seeking: slugify").unwrap();
    let o = g.open_asks(&alice(), "r").unwrap();
    assert_eq!((o.len(), o[0].sender.as_str()), (1, "bob"));
}

#[test]
fn a_digest_tells_the_asker_once_that_its_ask_has_no_claim() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.store.post("r", "alice", "seeking: a lib", 10).unwrap();
    let d = g.digest(&alice()).unwrap();
    assert_eq!(d.rooms[0].stale.len(), 1);
    assert!(
        g.digest(&alice()).unwrap().rooms[0].stale.is_empty(),
        "reported once"
    );
}

#[test]
fn a_young_or_claimed_ask_is_not_stale() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.store.post("r", "alice", "seeking: young", 90).unwrap();
    let old = g
        .store
        .post("r", "alice", "seeking: claimed", 10)
        .unwrap()
        .seq;
    g.claim(&bob(), "r", &format!("ask:{old}"), 500, "")
        .unwrap();
    assert!(g.digest(&alice()).unwrap().rooms[0].stale.is_empty());
}

#[test]
fn the_backlog_lists_others_open_unclaimed_asks_only_when_asked_for() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.store.post("r", "bob", "seeking: open", 10).unwrap();
    let claimed = g.store.post("r", "bob", "seeking: taken", 10).unwrap().seq;
    g.store.post("r", "alice", "seeking: mine", 10).unwrap();
    g.claim(&bob(), "r", &format!("ask:{claimed}"), 500, "")
        .unwrap();
    assert!(
        g.digest(&alice())
            .unwrap()
            .rooms
            .iter()
            .all(|r| r.backlog.is_empty())
    );
    let d = g.digest_with(&alice(), true).unwrap();
    let b = &d.rooms[0].backlog;
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].text, "seeking: open");
}

#[test]
fn the_board_combines_asks_offers_members_and_status() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.store.post("r", "bob", "seeking: slugify", 90).unwrap();
    g.store.post("r", "alice", "offering: helpers", 95).unwrap();
    g.store.post("r", "bob", "status: busy", 96).unwrap();
    let b = g.board(&alice(), "r").unwrap();
    assert_eq!(b.room, "r");
    assert_eq!(b.asks.len(), 1);
    assert_eq!(b.offers.len(), 1);
    assert_eq!(b.offers[0].sender, "alice");
    let bob = b.members.iter().find(|m| m.name == "bob").unwrap();
    assert_eq!(bob.status.as_deref(), Some("busy"));
    let alice = b.members.iter().find(|m| m.name == "alice").unwrap();
    assert_eq!(alice.status, None);
}

#[test]
fn a_board_is_refused_where_a_read_is_and_never_reaches_the_store() {
    let g = Gateway::new(
        SpyStore::new(),
        FixedClock(1),
        Rc::new(SpyGuard::denying(&["locked"])),
    );
    assert!(g.board(&alice(), "locked").is_err());
    assert_eq!(g.store.calls.get(), 0);
}

#[test]
fn the_board_lists_recent_members_and_counts_the_long_gone() {
    let g = Gateway::new(SpyStore::new(), FixedClock(1000), AllowAll);
    g.join(&alice(), "r").unwrap();
    g.store.join("r", "ghost", 1).unwrap();
    g.store.join("r", "bob", 999).unwrap();
    let b = g.board(&alice(), "r").unwrap();
    let names: Vec<_> = b.members.iter().map(|m| m.name.as_str()).collect();
    assert!(
        names.contains(&"alice") && names.contains(&"bob"),
        "{names:?}"
    );
    assert!(
        !names.contains(&"ghost"),
        "listed a member not seen for ages: {names:?}"
    );
    assert_eq!(b.stale_members, 1);
}

#[test]
fn reading_the_board_does_not_make_the_reader_a_member() {
    let g = open();
    g.join(&bob(), "r").unwrap();
    g.board(&alice(), "r").unwrap();
    let names: Vec<_> = g
        .who(&bob(), "r")
        .unwrap()
        .into_iter()
        .map(|m| m.name)
        .collect();
    assert!(!names.contains(&"alice".to_string()), "{names:?}");
}

#[test]
fn a_status_goes_through_the_guard_and_shows_in_who() {
    let g = Gateway::new(
        SpyStore::new(),
        FixedClock(5),
        Rc::new(SpyGuard::allowing()),
    );
    g.join(&alice(), "r").unwrap();
    g.set_status(&alice(), "r", "done leaving").unwrap();
    let m = g.who(&alice(), "r").unwrap();
    assert_eq!(m[0].status.as_deref(), Some("done leaving"));
}

#[test]
fn a_status_in_a_refused_room_never_reaches_the_store() {
    let g = Gateway::new(
        SpyStore::new(),
        FixedClock(1),
        Rc::new(SpyGuard::denying(&["locked"])),
    );
    assert!(g.set_status(&alice(), "locked", "done").is_err());
    assert_eq!(g.store.calls.get(), 0);
}

#[test]
fn a_bad_status_is_rejected_before_the_store() {
    let g = open();
    for bad in ["", "   ", "a\nb", "x\u{7}y", &"z".repeat(81)] {
        let err = g.set_status(&alice(), "r", bad).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{bad:?}: {err}");
    }
    assert_eq!(g.store.calls.get(), 0);
}

#[test]
fn the_board_prefers_the_status_fact_and_drops_the_offers_of_a_member_who_is_done() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.store.post("r", "bob", "offering: helpers", 90).unwrap();
    g.set_status(&bob(), "r", "done leaving").unwrap();
    let b = g.board(&alice(), "r").unwrap();
    let bob = b.members.iter().find(|m| m.name == "bob").unwrap();
    assert_eq!(bob.status.as_deref(), Some("done leaving"));
    assert!(
        b.offers.is_empty(),
        "a done member's offer stayed on the board"
    );
}

#[test]
fn an_offer_goes_when_its_sender_has_not_been_seen_for_a_while() {
    let g = Gateway::new(SpyStore::new(), FixedClock(1000), AllowAll);
    g.join(&alice(), "r").unwrap();
    g.store.post("r", "ghost", "offering: old help", 1).unwrap();
    g.store
        .post("r", "bob", "offering: fresh help", 999)
        .unwrap();
    let b = g.board(&alice(), "r").unwrap();
    let who: Vec<_> = b.offers.iter().map(|o| o.sender.as_str()).collect();
    assert_eq!(who, ["bob"], "an offer outlived its sender's presence");
}

#[test]
fn a_present_member_keeps_its_offer_without_renewing_it() {
    let g = Gateway::new(SpyStore::new(), FixedClock(5000), AllowAll);
    g.join(&alice(), "r").unwrap();
    g.store.post("r", "bob", "offering: help", 100).unwrap();
    g.store.wait_poll("r", "bob", 4990).unwrap();
    let b = g.board(&alice(), "r").unwrap();
    assert_eq!(
        b.offers.len(),
        1,
        "a waiting member lost its offer to the clock"
    );
}

#[test]
fn a_hook_run_is_recorded_for_the_caller_and_readable_by_anyone() {
    let g = open();
    g.record_hook_run(&alice(), "SessionStart", 0).unwrap();
    g.record_hook_run(&alice(), "SessionStart", 40).unwrap();
    let r = g.hook_runs(&bob(), "alice").unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(
        (
            r[0].event.as_str(),
            r[0].runs,
            r[0].last_bytes,
            r[0].last_ts
        ),
        ("SessionStart", 2, 40, 100)
    );
}

#[test]
fn a_bad_event_or_member_name_is_rejected_before_the_store() {
    let g = open();
    for bad in ["", "a b", "x\ny", &"e".repeat(41)] {
        assert!(
            matches!(g.record_hook_run(&alice(), bad, 0), Err(Error::Invalid(_))),
            "{bad:?}"
        );
    }
    assert!(matches!(
        g.hook_runs(&alice(), "bad name"),
        Err(Error::Invalid(_))
    ));
    assert_eq!(g.store.calls.get(), 0);
}

#[test]
fn pruning_goes_through_the_gateway_with_its_clock() {
    let g = Gateway::new(SpyStore::new(), FixedClock(10_000), AllowAll);
    g.store.join("r", "old", 1).unwrap();
    g.store.join("r", "fresh", 9_900).unwrap();
    assert_eq!(g.prune_members(&alice(), 5_000).unwrap(), 1);
    assert_eq!(g.who(&alice(), "r").unwrap().len(), 1);
}

#[test]
fn a_session_noted_by_a_member_is_remembered_as_that_member() {
    let g = open();
    g.note_session(&alice(), "sess-1").unwrap();
    assert_eq!(
        g.store.member_of_session("sess-1").unwrap().as_deref(),
        Some("alice")
    );
}

#[test]
fn a_bad_session_id_is_rejected_before_the_store() {
    let g = open();
    for bad in ["", "a b", "x\ny", &"s".repeat(81)] {
        assert!(
            matches!(g.note_session(&alice(), bad), Err(Error::Invalid(_))),
            "{bad:?}"
        );
    }
    assert_eq!(g.store.calls.get(), 0);
}

#[test]
fn two_scopes_can_claim_the_same_path_and_one_scope_cannot_claim_it_twice() {
    let g = open();
    let (a, b, c) = (alice(), bob(), Principal::unattributed("carol"));
    g.set_scope(&a, "git:a").unwrap();
    g.set_scope(&b, "git:b").unwrap();
    g.set_scope(&c, "git:a").unwrap();
    g.claim(&a, "r", "src/lib.rs", 100, "").unwrap();
    g.claim(&b, "r", "src/lib.rs", 100, "").unwrap();
    assert!(matches!(
        g.claim(&c, "r", "src/lib.rs", 100, ""),
        Err(Error::Conflict { .. })
    ));
    assert_eq!(g.claims(&a, "r").unwrap().len(), 2);
}

#[test]
fn a_shared_claim_and_an_ask_claim_conflict_across_scopes() {
    let g = open();
    let (a, b) = (alice(), bob());
    g.set_scope(&a, "git:a").unwrap();
    g.set_scope(&b, "git:b").unwrap();
    g.claim_with(&a, "r", "docs", 100, "", true).unwrap();
    assert!(
        g.claim(&b, "r", "docs/api.md", 100, "").is_err(),
        "a scoped claim against a shared one"
    );
    g.claim(&a, "r", "ask:3", 100, "").unwrap();
    assert!(
        g.claim(&b, "r", "ask:3", 100, "").is_err(),
        "an ask is room-wide"
    );
}

#[test]
fn members_with_no_scope_still_collide_as_before() {
    let g = open();
    g.claim(&alice(), "r", "p", 100, "").unwrap();
    assert!(g.claim(&bob(), "r", "p", 100, "").is_err());
}

#[test]
fn a_bad_scope_is_rejected_before_the_store() {
    let g = open();
    for bad in ["", "has space", "*", "new\nline", &"s".repeat(81)] {
        assert!(
            matches!(g.set_scope(&alice(), bad), Err(Error::Invalid(_))),
            "{bad:?}"
        );
    }
    assert_eq!(g.store.calls.get(), 0);
}

#[test]
fn an_ask_claim_lease_is_capped_and_a_digest_tells_the_asker_once_when_it_lapses() {
    let g = Gateway::new(SpyStore::new(), FixedClock(1000), AllowAll);
    g.join(&alice(), "r").unwrap();
    g.store.post("r", "alice", "seeking: a thing", 900).unwrap();
    g.claim(&bob(), "r", "ask:1", 3600, "").unwrap();
    let held = g.claims(&alice(), "r").unwrap();
    assert_eq!(
        held[0].expires,
        1000 + 300,
        "an ask lease is at most five minutes"
    );
    let g2 = Gateway::new(SpyStore::new(), FixedClock(1000), AllowAll);
    g2.join(&alice(), "r").unwrap();
    g2.store
        .post("r", "alice", "seeking: a thing", 900)
        .unwrap();
    g2.store
        .claim_in("r", "bob", "*", "ask:1", 90, "", 500)
        .unwrap(); // lapsed at 590
    let told = g2.digest(&alice()).unwrap();
    assert_eq!(told.rooms[0].lapsed.len(), 1);
    assert_eq!(told.rooms[0].lapsed[0].1, "bob");
    assert!(
        g2.digest(&alice()).unwrap().rooms[0].lapsed.is_empty(),
        "told once"
    );
}

#[test]
fn a_digest_shows_a_claim_once_not_on_every_call() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.claim(&bob(), "r", "src", 50, "").unwrap();
    assert_eq!(g.digest(&alice()).unwrap().rooms[0].claims.len(), 1);
    for _ in 0..3 {
        let d = g.digest(&alice()).unwrap();
        assert!(d.rooms[0].claims.is_empty(), "unchanged claim repeated");
        assert!(d.rooms[0].released.is_empty());
    }
}

#[test]
fn a_digest_reports_a_release_once() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.claim(&bob(), "r", "src", 50, "").unwrap();
    g.digest(&alice()).unwrap();
    g.release(&bob(), "r", "src").unwrap();
    let d = g.digest(&alice()).unwrap();
    assert_eq!(d.rooms[0].released.len(), 1);
    assert_eq!(d.rooms[0].released[0].owner, "bob");
    assert_eq!(d.rooms[0].released[0].path, "src");
    assert!(g.digest(&alice()).unwrap().rooms[0].released.is_empty());
}

#[test]
fn a_digest_still_delivers_messages_when_claims_are_unchanged() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.claim(&bob(), "r", "src", 50, "").unwrap();
    g.digest(&alice()).unwrap();
    g.post(&bob(), "r", "later").unwrap();
    let d = g.digest(&alice()).unwrap();
    assert_eq!(d.rooms[0].messages.len(), 1);
    assert!(d.rooms[0].claims.is_empty());
}

#[test]
fn a_digest_reports_refused_rooms_and_still_returns_the_others() {
    let g = Gateway::new(
        SpyStore::new(),
        FixedClock(1),
        Rc::new(SpyGuard::denying(&["secretroom"])),
    );
    let who = alice();
    g.join(&who, "open").unwrap();
    g.store.inner_join_for_test("secretroom", "alice");
    g.post(&bob(), "open", "visible").unwrap();
    let d = g.digest(&who).unwrap();
    assert_eq!(d.rooms.len(), 1);
    assert_eq!(d.rooms[0].room, "open");
    assert_eq!(d.refused.len(), 1);
    assert_eq!(d.refused[0].room, "secretroom");
    assert!(d.refused[0].reason.contains("test.deny_room"));
}
