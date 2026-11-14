use super::*;

/// A private room with a planted canary; returns the gateway and the canary's token.
fn planted() -> (Gateway<SpyStore>, String) {
    let g = open();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.admin_plant_canary(&admin(), "vault").unwrap();
    let body = g.tail(&alice(), "vault", (0, 10)).unwrap()[0].body.clone();
    let token = body.rsplit(' ').next().unwrap().to_string();
    (g, token)
}

#[test]
fn a_canary_is_a_message_from_the_canary_member_at_the_rooms_label() {
    let (g, token) = planted();
    let m = &g.tail(&alice(), "vault", (0, 10)).unwrap()[0];
    assert_eq!((m.sender.as_str(), m.label.as_str()), ("canary", "private"));
    assert!(token.starts_with("kc-canary-"), "{token}");
}

#[test]
fn a_canary_in_its_own_room_is_fine() {
    let (g, token) = planted();
    g.post(
        &alice(),
        "vault",
        &format!("quoting {token} for the record"),
    )
    .unwrap();
}

#[test]
fn a_canary_posted_elsewhere_is_refused_and_pauses_that_room() {
    let (g, token) = planted();
    let err = g
        .post(&alice(), "town", &format!("fyi {token}"))
        .unwrap_err();
    let msg = err.to_string();
    assert_eq!(rule_of(err), "canary");
    assert!(!msg.contains(&token), "{msg}");
    assert!(g.tail(&alice(), "town", (0, 10)).unwrap().is_empty());
    // the path is paused: the room takes nothing more until an admin reopens it, but stays readable
    assert_eq!(
        rule_of(g.post(&alice(), "town", "innocent").unwrap_err()),
        "tripwire"
    );
    assert_eq!(
        rule_of(
            g.claim(&bob(), "town", &ClaimSpec::new("p", 10, ""))
                .unwrap_err()
        ),
        "tripwire"
    );
    g.read(&bob(), "town", false).unwrap();
    g.admin_close_room(&admin(), "town", false).unwrap();
    g.post(&alice(), "town", "innocent").unwrap();
}

#[test]
fn the_trip_is_recorded_with_why_and_neither_record_holds_the_token() {
    let (g, token) = planted();
    let _ = g
        .post(&alice(), "town", &format!("fyi {token}"))
        .unwrap_err();
    let acts = g.admin_audit(&admin(), 5).unwrap();
    let trip = acts
        .iter()
        .find(|a| a.action == "tripwire")
        .expect("a tripwire act");
    assert_eq!(trip.target, "town");
    assert!(
        trip.detail.contains("vault") && trip.detail.contains("alice"),
        "{trip:?}"
    );
    assert!(!format!("{acts:?}").contains(&token));
    let refusals = g.admin_refusals(&admin(), 5).unwrap();
    assert_eq!(refusals[0].rule, "canary");
    assert!(!format!("{refusals:?}").contains(&token));
}

#[test]
fn a_canary_in_a_claim_reason_or_a_status_trips_too() {
    let (g, token) = planted();
    assert_eq!(
        rule_of(g.set_status(&alice(), "town", &token).unwrap_err()),
        "canary"
    );
    g.admin_close_room(&admin(), "town", false).unwrap();
    assert_eq!(
        rule_of(
            g.claim(&alice(), "town", &ClaimSpec::new("p", 10, &token))
                .unwrap_err()
        ),
        "canary"
    );
}

#[test]
fn a_direct_message_cannot_carry_a_canary_out() {
    let (g, token) = planted();
    assert_eq!(
        rule_of(g.dm(&alice(), "bob", &token, None).unwrap_err()),
        "canary"
    );
}

#[test]
fn a_member_with_five_safety_refusals_in_five_minutes_is_stopped_everywhere() {
    let (g, t) = timed();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    for i in 0..5 {
        t.set(t.get() + 10);
        let err = g.post(&bob(), "vault", &format!("try {i}")).unwrap_err();
        assert_eq!(rule_of(err), "clearance");
    }
    assert_eq!(
        rule_of(g.post(&bob(), "town", "innocent").unwrap_err()),
        "breaker"
    );
    assert_eq!(rule_of(g.join(&bob(), "other").unwrap_err()), "breaker");
    g.post(&alice(), "town", "others are unaffected").unwrap();
    let acts = g.admin_audit(&admin(), 5).unwrap();
    assert!(
        acts.iter()
            .any(|a| a.action == "breaker" && a.target == "bob"),
        "{acts:?}"
    );
    g.admin_allow_member(&admin(), "*", "bob").unwrap();
    g.post(&bob(), "town", "back").unwrap();
}

#[test]
fn refusals_that_are_not_about_safety_do_not_trip_the_breaker() {
    let (g, t) = timed();
    g.admin_close_room(&admin(), "r", true).unwrap();
    for i in 0..8 {
        t.set(t.get() + 1);
        assert_eq!(
            rule_of(g.post(&bob(), "r", &format!("x{i}")).unwrap_err()),
            "admin-close"
        );
    }
    g.post(&bob(), "elsewhere", "fine").unwrap();
}

#[test]
fn refusals_spread_over_more_than_the_window_do_not_trip_the_breaker() {
    let (g, t) = timed();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    for i in 0..8 {
        t.set(t.get() + 100);
        assert_eq!(
            rule_of(g.post(&bob(), "vault", &format!("try {i}")).unwrap_err()),
            "clearance"
        );
    }
    g.post(&bob(), "town", "still fine").unwrap();
}

#[test]
fn each_kill_switch_takes_effect_on_the_next_call() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.join(&bob(), "r").unwrap();
    g.post(&alice(), "r", "one").unwrap();
    // a room
    g.admin_close_room(&admin(), "r", true).unwrap();
    assert_eq!(
        rule_of(g.post(&alice(), "r", "two").unwrap_err()),
        "admin-close"
    );
    g.admin_close_room(&admin(), "r", false).unwrap();
    g.post(&alice(), "r", "two").unwrap();
    // a member
    g.admin_remove_member(&admin(), "r", "bob").unwrap();
    assert_eq!(
        rule_of(g.post(&bob(), "r", "x").unwrap_err()),
        "admin-block"
    );
    g.post(&alice(), "r", "three").unwrap();
    // everything
    g.admin_pause(&admin(), true).unwrap();
    assert_eq!(
        rule_of(g.post(&alice(), "r", "four").unwrap_err()),
        "admin-pause"
    );
    assert_eq!(
        rule_of(g.read(&alice(), "r", false).unwrap_err()),
        "admin-pause"
    );
    g.admin_pause(&admin(), false).unwrap();
    g.post(&alice(), "r", "four").unwrap();
}
