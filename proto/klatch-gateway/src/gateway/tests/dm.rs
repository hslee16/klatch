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
