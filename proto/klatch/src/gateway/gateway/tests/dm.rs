use super::*;

#[test]
fn a_direct_message_goes_to_a_room_both_members_are_in() {
    let g = open();
    let (room, _) = g.dm(&alice(), "bob", "hello bob", None).unwrap();
    assert_eq!(room, "dm.alice.bob");
    let got = g.read(&bob(), &room, false).unwrap();
    assert_eq!(got[0].body, "hello bob");
    let d = g.digest(&bob()).unwrap();
    assert_eq!(d.rooms[0].room, room, "the recipient's digest shows it");
    assert_eq!(g.dm(&bob(), "alice", "hi", None).unwrap().0, room);
}

#[test]
fn the_label_of_a_direct_message_room_is_the_lower_clearance_of_the_two() {
    let g = open();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.admin_set_clearance(&admin(), "bob", "internal").unwrap();
    let (room, _) = g.dm(&alice(), "bob", "x", None).unwrap();
    assert_eq!(g.room_label(&room).unwrap(), "internal");
    assert_eq!(
        g.tail(&alice(), &room, (0, 5)).unwrap()[0].label,
        "internal"
    );
}

#[test]
fn a_message_above_the_ceiling_is_refused_and_a_lower_label_is_allowed() {
    let g = open();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.admin_set_clearance(&admin(), "bob", "internal").unwrap();
    let err = g
        .dm(&alice(), "bob", "a private fact", Some("private"))
        .unwrap_err();
    assert_eq!(rule_of(err), "label");
    let (room, _) = g.dm(&alice(), "bob", "less so", Some("public")).unwrap();
    assert_eq!(g.tail(&alice(), &room, (0, 5)).unwrap()[0].label, "public");
}

#[test]
fn the_ceiling_follows_a_change_of_clearance() {
    let g = open();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.admin_set_clearance(&admin(), "bob", "private").unwrap();
    let (room, _) = g.dm(&alice(), "bob", "x", None).unwrap();
    assert_eq!(g.room_label(&room).unwrap(), "private");
    g.admin_set_clearance(&admin(), "bob", "public").unwrap();
    assert_eq!(g.room_label(&room).unwrap(), "public");
    assert_eq!(
        rule_of(g.dm(&alice(), "bob", "y", Some("private")).unwrap_err()),
        "label"
    );
}

#[test]
fn nobody_else_can_use_the_room() {
    let g = open();
    let (room, _) = g.dm(&alice(), "bob", "x", None).unwrap();
    let carol = Principal::unattributed("carol");
    for (name, err) in [
        ("join", g.join(&carol, &room).err()),
        ("read", g.read(&carol, &room, false).err()),
        ("post", g.post(&carol, &room, "hi").err()),
        ("who", g.who(&carol, &room).err()),
    ] {
        assert_eq!(
            rule_of(err.unwrap_or_else(|| panic!("{name} allowed"))),
            "dm-members",
            "{name}"
        );
    }
    assert_eq!(g.who(&alice(), &room).unwrap().len(), 2);
}

#[test]
fn a_dm_room_name_nobody_made_is_reserved() {
    let g = open();
    assert_eq!(
        rule_of(g.join(&alice(), "dm.alice.bob").unwrap_err()),
        "dm-reserved"
    );
    assert_eq!(
        rule_of(g.post(&alice(), "dm.x.y", "hi").unwrap_err()),
        "dm-reserved"
    );
}

#[test]
fn a_direct_message_to_oneself_or_a_bad_name_is_invalid() {
    let g = open();
    assert!(is_invalid(g.dm(&alice(), "alice", "x", None)));
    assert!(is_invalid(g.dm(&alice(), "bad name", "x", None)));
    assert!(is_invalid(g.dm(&alice(), "bob", "  ", None)));
}

#[test]
fn a_direct_message_passes_the_same_checks_as_a_post() {
    let g = open();
    assert_eq!(
        rule_of(
            g.dm(&alice(), "bob", "key AKIAIOSFODNN7EXAMPLE", None)
                .unwrap_err()
        ),
        "credentials"
    );
    g.admin_pause(&admin(), true).unwrap();
    assert_eq!(
        rule_of(g.dm(&alice(), "bob", "x", None).unwrap_err()),
        "admin-pause"
    );
}

#[test]
fn a_blocked_recipient_is_not_pulled_into_the_room() {
    let g = open();
    g.admin_remove_member(&admin(), "*", "bob").unwrap();
    let (room, _) = g.dm(&alice(), "bob", "x", None).unwrap();
    assert_eq!(
        rule_of(g.read(&bob(), &room, false).unwrap_err()),
        "admin-block"
    );
}

/// #1196: every member-reachable reader of a room refuses a third member of a direct message with `dm-members`.
#[test]
fn a_third_member_reads_a_dm_through_no_reader() {
    let g = open();
    let (room, _) = g.dm(&alice(), "bob", "the secret", None).unwrap();
    // a fresh third member for each reader, so one refusal never trips the breaker for the next
    let c = |n: &str| Principal::unattributed(n);
    let mut sub = crate::gateway::Subscription::new(&room, 0, 10);
    let results: Vec<(&str, Option<crate::domain::Error>)> = vec![
        ("tail", g.tail(&c("c1"), &room, (0, 10)).err()),
        ("tail_scan", g.tail_scan(&c("c2"), &room, (0, 10)).err()),
        ("head", g.head(&c("c3"), &room).err()),
        ("read", g.read(&c("c4"), &room, false).err()),
        ("wait_poll", g.wait_poll(&c("c5"), &room).err()),
        ("wait_seen", g.wait_seen(&c("c6"), &room, 1).err()),
        (
            "history",
            g.history(&c("c7"), &room, crate::gateway::HistoryRange::default())
                .err(),
        ),
        ("open_asks", g.open_asks(&c("c8"), &room).err()),
        ("stuck", g.stuck(&c("c9"), &room).err()),
        ("board", g.board(&c("c10"), &room).err()),
        ("receipt", g.receipt(&c("c11"), &room, 1).err()),
        ("subscription", sub.poll(&g, &c("c12"), 10).err()),
        (
            "request_crossing",
            g.request_crossing(&c("c13"), &room, 1, "town").err(),
        ),
    ];
    for (name, err) in results {
        assert_eq!(
            rule_of(err.unwrap_or_else(|| panic!("{name} allowed a third member"))),
            "dm-members",
            "{name}"
        );
    }
}

#[test]
fn the_two_members_still_tail_their_dm_and_a_plain_room_is_unchanged() {
    let g = open();
    let (room, _) = g.dm(&alice(), "bob", "hello", None).unwrap();
    assert_eq!(g.tail(&alice(), &room, (0, 10)).unwrap().len(), 1);
    assert_eq!(g.tail(&bob(), &room, (0, 10)).unwrap().len(), 1);
    let carol = Principal::unattributed("carol");
    g.join(&alice(), "town").unwrap();
    g.post(&alice(), "town", "hi").unwrap();
    assert_eq!(g.tail(&carol, "town", (0, 10)).unwrap().len(), 1);
}

#[test]
fn the_admin_path_reads_a_dm_and_a_member_does_not_have_it() {
    let g = open();
    let (room, _) = g.dm(&alice(), "bob", "hello", None).unwrap();
    // the human's floor act (export) reads any room through its own named path
    assert_eq!(
        g.admin_tail_scan(&admin(), &room, (0, 10)).unwrap().0.len(),
        1
    );
    // on a members' gateway the admin path is an admin act: a plain member is refused it, a dm or not
    let m = open().for_members();
    let (room, _) = m.dm(&alice(), "bob", "hello", None).unwrap();
    let carol = Principal::unattributed("carol");
    assert_eq!(
        rule_of(m.admin_tail_scan(&carol, &room, (0, 10)).unwrap_err()),
        "admin-scope"
    );
}
