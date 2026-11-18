//! #242: room settings. The human sets, per room, how long an ask may sit unclaimed before its asker is told (the
//! stale threshold) and the lease an `ask:<N>` claim gets when none is asked for. Both are admin-set policy keys with
//! bounds, the human's alone, audited, and neither can weaken D5 (a claim never lasts over 24 hours; an ask claim
//! never over its 300 second cap, whatever the setting).

use super::*;
use crate::domain::{ASK_LEASE_DEFAULT_SECS, ASK_LEASE_MAX_SECS, CLAIM_MAX_SECS, STALE_AFTER_SECS};

fn ada() -> Principal {
    Principal::unattributed("ada")
}

#[test]
fn a_room_has_the_built_in_values_until_the_human_sets_them() {
    let g = open();
    let s = g.settings_of("r").unwrap();
    assert_eq!(s.stale_secs, STALE_AFTER_SECS);
    assert_eq!(s.ask_lease_secs, ASK_LEASE_DEFAULT_SECS);
}

#[test]
fn the_human_sets_each_for_one_room_and_other_rooms_keep_the_defaults() {
    let g = open();
    g.admin_set_room_stale(&admin(), "slow", Some(600)).unwrap();
    g.admin_set_room_ask_lease(&admin(), "slow", Some(200))
        .unwrap();
    let slow = g.settings_of("slow").unwrap();
    assert_eq!((slow.stale_secs, slow.ask_lease_secs), (600, 200));
    let other = g.settings_of("lobby").unwrap();
    assert_eq!(
        (other.stale_secs, other.ask_lease_secs),
        (STALE_AFTER_SECS, ASK_LEASE_DEFAULT_SECS)
    );
    assert_eq!(
        g.store_for_tests().policy_get("room:slow:stale").unwrap(),
        Some("600".into())
    );
    assert_eq!(
        g.store_for_tests()
            .policy_get("room:slow:ask-lease")
            .unwrap(),
        Some("200".into())
    );
}

#[test]
fn off_puts_each_back_to_its_default() {
    let g = open();
    g.admin_set_room_stale(&admin(), "r", Some(600)).unwrap();
    g.admin_set_room_ask_lease(&admin(), "r", Some(200))
        .unwrap();
    g.admin_set_room_stale(&admin(), "r", None).unwrap();
    g.admin_set_room_ask_lease(&admin(), "r", None).unwrap();
    let s = g.settings_of("r").unwrap();
    assert_eq!(
        (s.stale_secs, s.ask_lease_secs),
        (STALE_AFTER_SECS, ASK_LEASE_DEFAULT_SECS)
    );
    assert_eq!(
        g.store_for_tests().policy_get("room:r:stale").unwrap(),
        None
    );
}

#[test]
fn the_stale_threshold_takes_ten_seconds_to_an_hour_and_nothing_outside() {
    let g = open();
    for ok in [10, 3_600] {
        g.admin_set_room_stale(&admin(), "r", Some(ok)).unwrap();
        assert_eq!(g.settings_of("r").unwrap().stale_secs, ok);
    }
    for bad in [i64::MIN, -1, 0, 9, 3_601, i64::MAX] {
        assert!(
            is_invalid(g.admin_set_room_stale(&admin(), "r", Some(bad))),
            "{bad}"
        );
        assert_eq!(g.settings_of("r").unwrap().stale_secs, 3_600, "{bad}");
    }
}

#[test]
fn the_ask_lease_takes_ten_seconds_to_the_ask_cap_and_nothing_outside() {
    let g = open();
    for ok in [10, ASK_LEASE_MAX_SECS] {
        g.admin_set_room_ask_lease(&admin(), "r", Some(ok)).unwrap();
        assert_eq!(g.settings_of("r").unwrap().ask_lease_secs, ok);
    }
    for bad in [
        i64::MIN,
        -1,
        0,
        9,
        ASK_LEASE_MAX_SECS + 1,
        CLAIM_MAX_SECS,
        i64::MAX,
    ] {
        assert!(
            is_invalid(g.admin_set_room_ask_lease(&admin(), "r", Some(bad))),
            "{bad}"
        );
        assert_eq!(
            g.settings_of("r").unwrap().ask_lease_secs,
            ASK_LEASE_MAX_SECS,
            "{bad}"
        );
    }
}

#[test]
fn a_bad_room_name_is_refused_and_stores_nothing() {
    let g = open();
    for bad in ["", "has space", "a:b", "new\nline"] {
        assert!(
            is_invalid(g.admin_set_room_stale(&admin(), bad, Some(60))),
            "{bad:?}"
        );
    }
    assert!(g.store_for_tests().policy_list("room:").unwrap().is_empty());
}

#[test]
fn an_admin_agent_and_a_member_may_not_set_either() {
    let g = open();
    g.admin_set_admin_agent(&admin(), "ada", true).unwrap();
    assert_eq!(
        rule_of(g.admin_set_room_stale(&ada(), "r", Some(600)).unwrap_err()),
        "admin-scope"
    );
    assert_eq!(
        rule_of(
            g.admin_set_room_ask_lease(&ada(), "r", Some(200))
                .unwrap_err()
        ),
        "admin-scope"
    );
    let s = g.settings_of("r").unwrap();
    assert_eq!(
        (s.stale_secs, s.ask_lease_secs),
        (STALE_AFTER_SECS, ASK_LEASE_DEFAULT_SECS)
    );
    let members = open().for_members();
    assert!(is_refused(members.admin_set_room_stale(
        &alice(),
        "r",
        Some(600)
    )));
}

#[test]
fn each_change_is_audited_with_the_room_and_its_new_value() {
    let g = open();
    g.admin_set_room_stale(&admin(), "slow", Some(600)).unwrap();
    g.admin_set_room_ask_lease(&admin(), "slow", None).unwrap();
    let rows = g.admin_audit(&admin(), 10).unwrap();
    assert!(
        rows.iter()
            .any(|r| r.action == "room-stale" && r.target == "slow 600"),
        "{rows:?}"
    );
    assert!(
        rows.iter()
            .any(|r| r.action == "room-ask-lease" && r.target == "slow off"),
        "{rows:?}"
    );
}

#[test]
fn a_stored_value_outside_the_bounds_reads_as_the_default() {
    let g = open();
    for junk in [
        "",
        "abc",
        "0",
        "-5",
        "1.5",
        "9",
        "3601",
        "99999999999999999999",
    ] {
        g.store_for_tests()
            .policy_set("room:r:stale", Some(junk), 0)
            .unwrap();
        assert_eq!(
            g.settings_of("r").unwrap().stale_secs,
            STALE_AFTER_SECS,
            "{junk:?}"
        );
    }
    for junk in ["", "abc", "0", "9", "301", "86400"] {
        g.store_for_tests()
            .policy_set("room:r:ask-lease", Some(junk), 0)
            .unwrap();
        assert_eq!(
            g.settings_of("r").unwrap().ask_lease_secs,
            ASK_LEASE_DEFAULT_SECS,
            "{junk:?}"
        );
    }
}

#[test]
fn the_digest_uses_the_rooms_stale_threshold() {
    let g = open();
    g.join(&alice(), "slow").unwrap();
    g.join(&alice(), "quick").unwrap();
    // each ask is 30 seconds old at now = 100
    for room in ["slow", "quick"] {
        g.store
            .post(Seat::new(room, "alice"), "seeking: a lib", 70)
            .unwrap();
    }
    g.admin_set_room_stale(&admin(), "quick", Some(10)).unwrap();
    let d = g.digest(&alice()).unwrap();
    let stale_in = |room: &str| d.rooms.iter().find(|r| r.room == room).unwrap().stale.len();
    assert_eq!(stale_in("quick"), 1, "30 s old is stale past 10 s");
    assert_eq!(
        stale_in("slow"),
        0,
        "30 s old is not stale at the default 60 s"
    );
}

#[test]
fn a_longer_threshold_holds_an_ask_back_that_the_default_would_report() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.store
        .post(Seat::new("r", "alice"), "seeking: a lib", 10)
        .unwrap();
    g.admin_set_room_stale(&admin(), "r", Some(300)).unwrap();
    assert!(
        g.digest(&alice()).unwrap().rooms[0].stale.is_empty(),
        "90 s old, the room waits 300 s"
    );
}

#[test]
fn an_ask_claim_without_a_requested_lease_gets_the_rooms_default_and_others_do_not() {
    let g = open();
    g.admin_set_room_ask_lease(&admin(), "r", Some(200))
        .unwrap();
    let ttl = |room, path, asked| g.claim_ttl_for(&alice(), (room, path), asked).unwrap();
    assert_eq!(ttl("r", "ask:3", None), 200);
    assert_eq!(ttl("other", "ask:3", None), ASK_LEASE_DEFAULT_SECS);
    assert_eq!(ttl("r", "src/lib.rs", None), 600);
}

#[test]
fn a_requested_lease_is_kept_and_an_ask_claim_never_passes_its_cap_whatever_the_setting() {
    let g = open();
    g.admin_set_room_ask_lease(&admin(), "r", Some(ASK_LEASE_MAX_SECS))
        .unwrap();
    let ttl = |path, asked| g.claim_ttl_for(&alice(), ("r", path), asked).unwrap();
    assert_eq!(ttl("ask:3", Some(120)), 120);
    assert_eq!(ttl("ask:3", Some(3_600)), ASK_LEASE_MAX_SECS);
    assert_eq!(ttl("src/lib.rs", Some(3_600)), 3_600);
    // and the gateway still refuses a claim over 24 hours, whatever a room sets
    let over = ClaimSpec::new("src/lib.rs", CLAIM_MAX_SECS + 1, "");
    assert!(is_invalid(g.claim(&alice(), "r", &over)));
}

#[test]
fn a_claim_taken_with_the_rooms_default_lapses_when_that_lease_ends() {
    let (g, clock) = timed();
    g.admin_set_room_ask_lease(&admin(), "r", Some(20)).unwrap();
    g.join(&alice(), "r").unwrap();
    let ttl = g.claim_ttl_for(&alice(), ("r", "ask:1"), None).unwrap();
    g.claim(&alice(), "r", &ClaimSpec::new("ask:1", ttl, ""))
        .unwrap();
    assert_eq!(g.claims(&alice(), "r").unwrap().len(), 1);
    clock.set(clock.get() + 21);
    assert!(g.claims(&alice(), "r").unwrap().is_empty());
}
