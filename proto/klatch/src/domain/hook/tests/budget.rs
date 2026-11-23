use super::*;

#[test]
fn when_messages_do_not_fit_the_newest_are_kept_and_the_older_ones_counted() {
    let d = RoomDigest {
        room: "lobby".into(),
        messages: thirty(),
        ..Default::default()
    };
    let out = render_hook(&[d], 0);
    shows_newest_down_to!(out, 30, 11);
    shows!(out, "[10 more messages not shown;");
}

#[test]
fn an_addressed_message_is_kept_over_a_newer_one_and_shown_in_order() {
    let d = RoomDigest {
        room: "lobby".into(),
        messages: thirty(),
        addressed: vec![3],
        ..Default::default()
    };
    let out = render_hook(&[d], 0);
    shows!(out, "#3 from bob");
    shows_newest_down_to!(out, 12, 12);
    let order = (out.find("#3 from"), out.find("#12 from"));
    assert!(order.0.unwrap() < order.1.unwrap(), "{out}");
    shows!(out, "[10 more messages not shown;");
}

#[test]
fn more_addressed_messages_than_the_cap_keep_the_newest_addressed() {
    let d = RoomDigest {
        room: "lobby".into(),
        messages: thirty(),
        addressed: (1..=30).collect(),
        ..Default::default()
    };
    let out = render_hook(&[d], 0);
    assert!(
        out.contains("#30 from bob") && out.contains("#11 from bob"),
        "{out}"
    );
    assert!(!out.contains("#10 from bob"), "{out}");
}

#[test]
fn under_a_tight_byte_budget_the_addressed_message_still_comes_first() {
    let mut messages = many(20, 900);
    messages[0] = msg(1, "@me please look at the first one");
    let d = RoomDigest {
        room: "lobby".into(),
        messages,
        addressed: vec![1],
        ..Default::default()
    };
    let out = render_hook(&[d], 0);
    assert!(out.contains("@me please look at the first one"), "{out}");
    assert!(out.contains("#20 from bob"), "{out}");
}

#[test]
fn an_ask_for_the_member_is_kept_over_newer_addressed_messages_when_few_fit() {
    use crate::domain::Importance;
    let d = RoomDigest {
        room: "lobby".into(),
        messages: thirty(),
        // #1 is an open ask the member could answer: not addressed to it, but the most important class (#1405)
        addressed: vec![25, 26, 27, 28],
        ranks: vec![
            (1, Importance::Ask),
            (25, Importance::Addressed),
            (26, Importance::Addressed),
            (27, Importance::Addressed),
            (28, Importance::Addressed),
        ],
        ..Default::default()
    };
    let out = render_hook_at(&[d], 0, BudgetLevel::Few);
    shows!(out, "#1 from bob");
    shows!(out, "#28 from bob");
    shows!(out, "#27 from bob");
    assert!(!out.contains("#26 from bob"), "{out}");
    assert!(!out.contains("#30 from bob"), "{out}");
}

#[test]
fn an_older_direct_message_ranks_above_newer_addressed_messages_when_few_fit() {
    use crate::domain::Importance;
    let d = RoomDigest {
        room: "lobby".into(),
        messages: thirty(),
        addressed: vec![2, 3, 4, 20],
        ranks: vec![
            (2, Importance::Dm),
            (3, Importance::Addressed),
            (4, Importance::Addressed),
            (20, Importance::Addressed),
        ],
        ..Default::default()
    };
    let out = render_hook_at(&[d], 0, BudgetLevel::Few);
    shows!(out, "#2 from bob");
    shows!(out, "#20 from bob");
    shows!(out, "#4 from bob");
    assert!(!out.contains("#3 from bob"), "{out}");
}

#[test]
fn the_room_with_a_direct_message_prints_before_a_room_with_newer_chatter() {
    use crate::domain::Importance;
    let lobby = RoomDigest {
        room: "lobby".into(),
        messages: vec![Message {
            ts: 500,
            ..msg(1, "chatter")
        }],
        ..Default::default()
    };
    let dm = RoomDigest {
        room: "dm.a.b".into(),
        messages: vec![Message {
            ts: 5,
            ..msg(1, "hello")
        }],
        addressed: vec![1],
        ranks: vec![(1, Importance::Dm)],
        ..Default::default()
    };
    let out = render_hook(&[lobby, dm], 1000);
    let (a, b) = (out.find("[room dm.a.b]"), out.find("[room lobby]"));
    assert!(a.unwrap() < b.unwrap(), "{out}");
}

#[test]
fn at_the_count_only_level_a_rank_alone_is_not_enough_to_print_a_message() {
    // never-drop stays what #1419 decided: addressed messages; an open ask that is not addressed is counted
    use crate::domain::Importance;
    let d = RoomDigest {
        room: "lobby".into(),
        messages: thirty(),
        addressed: vec![9],
        ranks: vec![(1, Importance::Ask), (9, Importance::Addressed)],
        ..Default::default()
    };
    let out = render_hook_at(&[d], 0, BudgetLevel::CountOnly);
    shows!(out, "#9 from bob");
    assert!(!out.contains("#1 from bob"), "{out}");
}

#[test]
fn a_digest_never_prints_more_than_the_byte_budget_however_much_is_unread() {
    use crate::domain::budget::DIGEST_MAX_BYTES;
    let d = RoomDigest {
        room: "lobby".into(),
        messages: many(20, 900),
        ..Default::default()
    };
    let out = render_hook(&[d], 0);
    assert!(printed_bytes(&out) <= DIGEST_MAX_BYTES, "{}", out.len());
}

#[test]
fn what_is_cut_for_size_is_counted_in_one_line_that_says_how_to_read_it() {
    let d = RoomDigest {
        room: "lobby".into(),
        messages: many(20, 900),
        ..Default::default()
    };
    let out = render_hook(&[d], 0);
    let shown = out.matches(" from bob").count();
    assert!((1..20).contains(&shown), "{shown}");
    shows!(
        out,
        &format!(
            "{} more messages not shown; run `klatch read lobby --peek`",
            20 - shown
        ),
    );
    // the newest are kept, not the oldest
    shows!(out, "#20 from bob");
    shows!(out, &format!("#{} from bob", 20 - shown + 1));
}

#[test]
fn a_message_is_shown_whole_or_not_at_all_and_never_cut_inside() {
    let d = RoomDigest {
        room: "lobby".into(),
        messages: many(20, 900),
        ..Default::default()
    };
    let out = render_hook(&[d], 0);
    for line in out.lines().filter(|l| l.contains(" from bob")) {
        assert!(
            line.ends_with(&"y".repeat(900)),
            "cut inside a message: {line:?}"
        );
    }
    for line in out.lines() {
        assert!(
            line.starts_with("[room ") || line.starts_with("  "),
            "an unfenced line: {line:?}"
        );
    }
}

#[test]
fn escaping_counts_toward_the_budget() {
    // quotes double in JSON, so a body of quotes must be measured after escaping
    use crate::domain::budget::DIGEST_MAX_BYTES;
    let d = RoomDigest {
        room: "lobby".into(),
        messages: (1..=20).map(|i| msg(i, &"\"".repeat(900))).collect(),
        ..Default::default()
    };
    assert!(printed_bytes(&render_hook(&[d], 0)) <= DIGEST_MAX_BYTES);
}

#[test]
fn several_rooms_share_one_budget_and_claims_are_not_starved_by_messages() {
    use crate::domain::budget::DIGEST_MAX_BYTES;
    let claim = Claim {
        path: "src/a.rs".into(),
        owner: "carol".into(),
        reason: String::new(),
        expires: 100,
        scope: String::new(),
    };
    let rooms: Vec<RoomDigest> = ["a", "b", "c"]
        .iter()
        .map(|r| RoomDigest {
            room: (*r).into(),
            messages: many(20, 900),
            claims: vec![claim.clone()],
            ..Default::default()
        })
        .collect();
    let out = render_hook(&rooms, 0);
    assert!(printed_bytes(&out) <= DIGEST_MAX_BYTES);
    assert_eq!(out.matches("carol holds src/a.rs").count(), 3, "{out}");
    for r in ["a", "b", "c"] {
        assert!(out.contains(&format!("[room {r}]")), "{out}");
    }
}

#[test]
fn a_flood_of_claims_in_the_first_room_leaves_later_rooms_their_header_and_counts() {
    // found by the property test (RESEARCH-NOTES 42.8): the first room's claims used the whole budget, so the
    // headers and "more" lines of the rooms after it were printed on top of it
    use crate::domain::budget::DIGEST_MAX_BYTES;
    let claims: Vec<Claim> = (0..200)
        .map(|i| Claim {
            path: format!("src/some/long/path/number/{i}/file.rs"),
            owner: "carol".into(),
            reason: String::new(),
            expires: 100,
            scope: String::new(),
        })
        .collect();
    let long = "r".repeat(40);
    let rooms: Vec<RoomDigest> = (0..5)
        .map(|i| RoomDigest {
            room: format!("{long}{i}").chars().take(40).collect(),
            claims: claims.clone(),
            messages: many(20, 900),
            ..Default::default()
        })
        .collect();
    let out = render_hook(&rooms, 0);
    assert!(
        printed_bytes(&out) <= DIGEST_MAX_BYTES,
        "{} bytes",
        printed_bytes(&out)
    );
    assert_eq!(
        out.matches("[room ").count(),
        5,
        "every room keeps a header"
    );
}
