use super::*;

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
