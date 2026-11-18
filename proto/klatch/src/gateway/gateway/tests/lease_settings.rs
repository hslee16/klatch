//! #537: the human sets the lease cap and grace period with an admin act instead of writing the policy keys by hand.
//! Both are the human's alone (an admin agent is refused), both are audited, both take a bounded whole
//! number of seconds or are removed (back to the default), and a nonsense value changes nothing.

use super::*;
use crate::gateway::gateway::lease::{DEFAULT_CAP_SECS, DEFAULT_GRACE_SECS};

fn ada() -> Principal {
    Principal::unattributed("ada")
}

#[test]
fn the_cap_and_the_grace_default_until_the_human_sets_them() {
    let (g, _) = timed();
    let p = g.lease_policy().unwrap();
    assert_eq!(p.cap_secs, DEFAULT_CAP_SECS);
    assert_eq!(p.grace_secs, DEFAULT_GRACE_SECS);
}

#[test]
fn the_human_sets_the_cap_and_it_is_what_the_lease_rules_read() {
    let (g, _) = timed();
    g.admin_set_lease_cap(&admin(), Some(86_400)).unwrap();
    assert_eq!(g.lease_policy().unwrap().cap_secs, 86_400);
    assert_eq!(g.lease_policy().unwrap().grace_secs, DEFAULT_GRACE_SECS);
    assert_eq!(
        g.store_for_tests().policy_get("lease:cap").unwrap(),
        Some("86400".into())
    );
}

#[test]
fn the_human_sets_the_grace_and_it_is_what_the_lease_rules_read() {
    let (g, _) = timed();
    g.admin_set_lease_grace(&admin(), Some(60)).unwrap();
    assert_eq!(g.lease_policy().unwrap().grace_secs, 60);
    assert_eq!(g.lease_policy().unwrap().cap_secs, DEFAULT_CAP_SECS);
    assert_eq!(
        g.store_for_tests().policy_get("lease:grace").unwrap(),
        Some("60".into())
    );
}

#[test]
fn off_puts_each_back_to_its_default() {
    let (g, _) = timed();
    g.admin_set_lease_cap(&admin(), Some(7_200)).unwrap();
    g.admin_set_lease_grace(&admin(), Some(30)).unwrap();
    g.admin_set_lease_cap(&admin(), None).unwrap();
    g.admin_set_lease_grace(&admin(), None).unwrap();
    let p = g.lease_policy().unwrap();
    assert_eq!(
        (p.cap_secs, p.grace_secs),
        (DEFAULT_CAP_SECS, DEFAULT_GRACE_SECS)
    );
    assert_eq!(g.store_for_tests().policy_get("lease:cap").unwrap(), None);
    assert_eq!(g.store_for_tests().policy_get("lease:grace").unwrap(), None);
}

#[test]
fn the_cap_takes_one_second_to_seven_days_and_nothing_outside() {
    let (g, _) = timed();
    for ok in [1, 604_800] {
        g.admin_set_lease_cap(&admin(), Some(ok)).unwrap();
        assert_eq!(g.lease_policy().unwrap().cap_secs, ok);
    }
    for bad in [i64::MIN, -1, 0, 604_801, i64::MAX] {
        assert!(
            is_invalid(g.admin_set_lease_cap(&admin(), Some(bad))),
            "{bad}"
        );
        assert_eq!(
            g.lease_policy().unwrap().cap_secs,
            604_800,
            "{bad} changed it"
        );
    }
}

#[test]
fn the_grace_takes_one_second_to_a_day_and_nothing_outside() {
    let (g, _) = timed();
    for ok in [1, 86_400] {
        g.admin_set_lease_grace(&admin(), Some(ok)).unwrap();
        assert_eq!(g.lease_policy().unwrap().grace_secs, ok);
    }
    for bad in [i64::MIN, -1, 0, 86_401, i64::MAX] {
        assert!(
            is_invalid(g.admin_set_lease_grace(&admin(), Some(bad))),
            "{bad}"
        );
        assert_eq!(
            g.lease_policy().unwrap().grace_secs,
            86_400,
            "{bad} changed it"
        );
    }
}

#[test]
fn an_admin_agent_and_a_member_may_not_set_either() {
    let (g, _) = timed();
    g.admin_set_admin_agent(&admin(), "ada", true).unwrap();
    assert_eq!(
        rule_of(g.admin_set_lease_cap(&ada(), Some(86_400)).unwrap_err()),
        "admin-scope"
    );
    assert_eq!(
        rule_of(g.admin_set_lease_grace(&ada(), Some(60)).unwrap_err()),
        "admin-scope"
    );
    let p = g.lease_policy().unwrap();
    assert_eq!(
        (p.cap_secs, p.grace_secs),
        (DEFAULT_CAP_SECS, DEFAULT_GRACE_SECS)
    );
}

#[test]
fn each_change_is_audited_with_its_new_value() {
    let (g, _) = timed();
    g.admin_set_lease_cap(&admin(), Some(7_200)).unwrap();
    g.admin_set_lease_grace(&admin(), None).unwrap();
    let rows = g.admin_audit(&admin(), 10).unwrap();
    assert!(
        rows.iter()
            .any(|r| r.action == "lease-cap" && r.target == "7200"),
        "{rows:?}"
    );
    assert!(
        rows.iter()
            .any(|r| r.action == "lease-grace" && r.target == "off"),
        "{rows:?}"
    );
}

#[test]
fn a_value_that_is_not_a_positive_number_in_the_store_still_reads_as_the_default() {
    let (g, _) = timed();
    for junk in ["", "abc", "0", "-5", "1.5"] {
        g.store_for_tests()
            .policy_set("lease:cap", Some(junk), 0)
            .unwrap();
        assert_eq!(
            g.lease_policy().unwrap().cap_secs,
            DEFAULT_CAP_SECS,
            "{junk:?}"
        );
    }
}
