//! #187: a rate limit per member at the gateway, over every room the member posts in (the receiver budget, R22, is per
//! room). Off until the human sets one; a refusal names the rule and says how long to wait; other members are
//! unaffected; the clock is the injected one.

use super::*;

fn ada() -> Principal {
    Principal::unattributed("ada")
}

/// Three posts in 60 seconds across all rooms, set by the human.
fn limited() -> (Gateway<SpyStore>, Rc<std::cell::Cell<i64>>) {
    let (g, t) = timed();
    g.admin_set_rate_limit(&admin(), Some((3, 60))).unwrap();
    (g, t)
}

/// Post `n` times from `who`, one second apart, each in a different room so the per-room budget never answers.
fn post_n(g: &Gateway<SpyStore>, t: &Rc<std::cell::Cell<i64>>, who: &Principal, n: usize) {
    for i in 0..n {
        g.post(who, &format!("room-{i}"), &format!("message {i}"))
            .unwrap();
        t.set(t.get() + 1);
    }
}

#[test]
fn with_no_limit_set_a_member_posts_as_before() {
    let (g, t) = timed();
    post_n(&g, &t, &alice(), 30);
}

#[test]
fn a_member_over_its_limit_is_refused_with_the_rule_and_how_long_to_wait() {
    let (g, t) = limited();
    post_n(&g, &t, &alice(), 3);
    let e = g.post(&alice(), "elsewhere", "one too many").unwrap_err();
    match e {
        Error::Refused { rule, reason } => {
            assert_eq!(rule, "rate-limit");
            assert!(reason.contains("3 posts in 60 seconds"), "{reason}");
            assert!(reason.contains("wait 57s"), "{reason}");
        }
        other => panic!("not refused: {other:?}"),
    }
}

#[test]
fn the_limit_counts_every_room_not_one() {
    let (g, t) = limited();
    post_n(&g, &t, &alice(), 3);
    assert_eq!(
        rule_of(
            g.post(&alice(), "room-0", "a fourth, in a room it used")
                .unwrap_err()
        ),
        "rate-limit"
    );
}

#[test]
fn another_member_is_unaffected() {
    let (g, t) = limited();
    post_n(&g, &t, &alice(), 3);
    assert!(g.post(&alice(), "x", "refused").is_err());
    g.post(&bob(), "x", "bob is fine").unwrap();
}

#[test]
fn the_window_moves_with_the_injected_clock() {
    let (g, t) = limited();
    post_n(&g, &t, &alice(), 3);
    assert!(g.post(&alice(), "x", "refused").is_err());
    t.set(t.get() + 60);
    g.post(&alice(), "x", "allowed again").unwrap();
}

#[test]
fn a_refused_post_is_not_stored_and_is_kept_in_the_refusals() {
    let (g, t) = limited();
    post_n(&g, &t, &alice(), 3);
    let _ = g.post(&alice(), "fresh", "refused");
    assert_eq!(g.head(&bob(), "fresh").unwrap(), 0);
    let refusals = g.admin_refusals(&admin(), 10).unwrap();
    assert!(
        refusals
            .iter()
            .any(|r| r.member == "alice" && r.rule == "rate-limit"),
        "{refusals:?}"
    );
}

#[test]
fn the_rate_limit_is_not_a_safety_rule_so_it_never_trips_the_breaker() {
    let (g, t) = limited();
    post_n(&g, &t, &alice(), 3);
    for _ in 0..8 {
        let _ = g.post(&alice(), "x", "refused");
    }
    t.set(t.get() + 60);
    g.post(&alice(), "x", "still allowed after the window")
        .unwrap();
}

#[test]
fn only_the_human_sets_the_limit_and_it_can_be_removed() {
    let (g, t) = limited();
    g.admin_set_admin_agent(&admin(), "ada", true).unwrap();
    assert_eq!(
        rule_of(g.admin_set_rate_limit(&ada(), None).unwrap_err()),
        "admin-scope"
    );
    post_n(&g, &t, &alice(), 3);
    g.admin_set_rate_limit(&admin(), None).unwrap();
    g.post(&alice(), "x", "no limit now").unwrap();
}

#[test]
fn a_nonsense_limit_is_refused() {
    let (g, _) = timed();
    for bad in [(0, 60), (3, 0), (3, 100_000), (100_000, 60)] {
        assert!(
            is_invalid(g.admin_set_rate_limit(&admin(), Some(bad))),
            "{bad:?}"
        );
    }
}

// #972: a limit for one member, set next to the global one; the member's own limit wins, and with none the
// global limit (or no limit) applies.

#[test]
fn a_member_limit_applies_to_that_member_with_no_global_limit() {
    let (g, t) = timed();
    g.admin_set_member_rate_limit(&admin(), "alice", Some((2, 60)))
        .unwrap();
    post_n(&g, &t, &alice(), 2);
    assert_eq!(
        rule_of(g.post(&alice(), "x", "third").unwrap_err()),
        "rate-limit"
    );
    post_n(&g, &t, &bob(), 30);
}

#[test]
fn a_member_limit_wins_over_the_global_one_in_both_directions() {
    let (g, t) = limited();
    g.admin_set_member_rate_limit(&admin(), "alice", Some((5, 60)))
        .unwrap();
    g.admin_set_member_rate_limit(&admin(), "bob", Some((1, 60)))
        .unwrap();
    post_n(&g, &t, &alice(), 5);
    assert!(g.post(&alice(), "x", "sixth").is_err());
    g.post(&bob(), "y", "one").unwrap();
    let e = g.post(&bob(), "y", "two").unwrap_err();
    assert!(
        matches!(&e, Error::Refused { reason, .. } if reason.contains("1 posts in 60 seconds")),
        "{e:?}"
    );
}

#[test]
fn a_member_without_its_own_limit_inherits_the_global_one() {
    let (g, t) = limited();
    g.admin_set_member_rate_limit(&admin(), "alice", Some((5, 60)))
        .unwrap();
    post_n(&g, &t, &bob(), 3);
    assert!(g.post(&bob(), "x", "fourth").is_err());
}

#[test]
fn removing_a_member_limit_goes_back_to_inheriting() {
    let (g, t) = limited();
    g.admin_set_member_rate_limit(&admin(), "alice", Some((1, 60)))
        .unwrap();
    assert_eq!(g.rate_limit_for("alice").unwrap(), Some((1, 60)));
    g.admin_set_member_rate_limit(&admin(), "alice", None)
        .unwrap();
    assert_eq!(g.rate_limit_for("alice").unwrap(), Some((3, 60)));
    post_n(&g, &t, &alice(), 3);
}

#[test]
fn a_member_limit_is_audited_and_validated() {
    let (g, _) = timed();
    for bad in [(0, 60), (3, 0), (3, 100_000), (100_000, 60)] {
        assert!(
            is_invalid(g.admin_set_member_rate_limit(&admin(), "alice", Some(bad))),
            "{bad:?}"
        );
    }
    assert!(is_invalid(g.admin_set_member_rate_limit(
        &admin(),
        "bad name!",
        Some((3, 60))
    )));
    g.admin_set_member_rate_limit(&admin(), "alice", Some((3, 60)))
        .unwrap();
    let audit = g.admin_audit(&admin(), 10).unwrap();
    assert!(
        audit.iter().any(|a| a.action == "member-rate-limit"),
        "{audit:?}"
    );
}

#[test]
fn a_member_limit_is_a_floor_no_agent_can_set_or_lift() {
    let (g, _) = timed();
    g.admin_set_admin_agent(&admin(), "ada", true).unwrap();
    assert_eq!(
        rule_of(
            g.admin_set_member_rate_limit(&ada(), "alice", Some((9, 60)))
                .unwrap_err()
        ),
        "floors"
    );
    let agent = Gateway::new(
        SpyStore::new(),
        FixedClock(100),
        crate::gateway::ForbidFloors::new(AllowAll),
    );
    assert_eq!(
        rule_of(
            agent
                .admin_set_member_rate_limit(&alice(), "alice", None)
                .unwrap_err()
        ),
        "floors"
    );
}
