use super::*;

#[test]
fn who_lists_recent_members_with_status_and_counts_the_long_gone() {
    let m = [
        peer("fresh", 990, Some("busy")),
        peer("old-1", 1, None),
        peer("old-2", 2, Some("done session ended")),
    ];
    let out = render_who(&m, 1000, false);
    assert!(
        out.contains("fresh\tlast seen 10s ago (liveness, not progress)\tstatus: busy"),
        "{out}"
    );
    assert!(!out.contains("old-1") && !out.contains("old-2"), "{out}");
    assert!(
        out.contains("2 others not seen for over 10 minutes (who --all lists them)"),
        "{out}"
    );
}

#[test]
fn no_age_is_printed_negative_when_the_reader_clock_is_behind() {
    let m = [peer("alice", 1500, Some("busy"))];
    let who = render_who(&m, 1000, false);
    assert!(
        who.contains("last seen 0s ago") && !who.contains("-"),
        "{who}"
    );
    let board = Board {
        room: "lobby".into(),
        asks: vec![OpenAsk {
            seq: 1,
            sender: "bob".into(),
            text: "seeking: x".into(),
            ts: 1500,
            ..Default::default()
        }],
        offers: vec![Offer {
            sender: "carol".into(),
            seq: 2,
            text: "offering: y".into(),
            ts: 1400,
        }],
        members: vec![BoardMember {
            status_ts: 0,
            name: "alice".into(),
            last_seen: 1500,
            status: Some("busy".into()),
            parent: None,
            parked: None,
            kind: None,
            place: None,
            advisory: false,
        }],
        ..Default::default()
    };
    let out = render_board(&board, 1000);
    assert!(!out.contains("-"), "a negative age was printed: {out}");
    assert!(
        out.contains("unclaimed 0s") && out.contains("(0s ago)"),
        "{out}"
    );
}

#[test]
fn a_child_session_is_shown_with_its_parent_in_who_and_on_the_board() {
    let mut child = peer("worker", 990, None);
    child.parent = Some("lead".into());
    let who = render_who(&[child, peer("lead", 990, None)], 1000, false);
    assert!(
        who.contains("worker\tlast seen 10s ago (liveness, not progress)\tchild of lead"),
        "{who}"
    );
    let line = who.lines().find(|l| l.starts_with("lead")).unwrap();
    assert!(!line.contains("child of"), "{line}");
    let board = Board {
        room: "lobby".into(),
        members: vec![BoardMember {
            status_ts: 0,
            name: "worker".into(),
            last_seen: 990,
            status: None,
            parent: Some("lead".into()),
            parked: None,
            kind: None,
            place: None,
            advisory: false,
        }],
        ..Default::default()
    };
    let text = render_board(&board, 1000);
    assert!(
        text.contains("worker last seen 10s ago, child of lead"),
        "{text}"
    );
}

#[test]
fn a_members_kind_is_shown_in_who_and_on_the_board_and_a_member_without_one_shows_none() {
    let mut bot = peer("grok", 990, None);
    bot.kind = Some(crate::domain::MemberKind::Bot);
    let who = render_who(&[bot, peer("plain", 990, None)], 1000, false);
    let grok = who.lines().find(|l| l.starts_with("grok")).unwrap();
    assert!(grok.contains("\tkind: bot"), "{grok}");
    let plain = who.lines().find(|l| l.starts_with("plain")).unwrap();
    assert!(!plain.contains("kind"), "{plain}");
    let board = Board {
        room: "lobby".into(),
        members: vec![BoardMember {
            status_ts: 0,
            name: "grok".into(),
            last_seen: 990,
            status: Some("busy".into()),
            parent: None,
            parked: None,
            kind: Some(crate::domain::MemberKind::Service),
            place: None,
            advisory: false,
        }],
        ..Default::default()
    };
    let text = render_board(&board, 1000);
    assert!(
        text.contains("grok last seen 10s ago, kind: service, status: busy"),
        "{text}"
    );
}

#[test]
fn a_kind_never_carries_text_a_member_wrote() {
    // the kind is a closed vocabulary rendered from an enum, so no member text can reach the line
    for k in [
        crate::domain::MemberKind::Session,
        crate::domain::MemberKind::Bot,
        crate::domain::MemberKind::Service,
        crate::domain::MemberKind::Human,
    ] {
        let mut m = peer("x", 999, None);
        m.kind = Some(k);
        let who = render_who(&[m], 1000, false);
        assert!(who.contains(&format!("\tkind: {}", k.word())), "{who}");
        assert_eq!(who.lines().count(), 1);
    }
}

#[test]
fn who_all_lists_everyone_and_says_nothing_about_hiding() {
    let m = [peer("fresh", 990, None), peer("old", 1, None)];
    let out = render_who(&m, 1000, true);
    assert!(
        out.contains("fresh") && out.contains("old\tlast seen 999s ago"),
        "{out}"
    );
    assert!(!out.contains("not seen for over"), "{out}");
}

#[test]
fn who_with_nobody_gone_has_no_count_line_and_one_gone_is_singular() {
    let out = render_who(&[peer("a", 999, None)], 1000, false);
    assert!(!out.contains("not seen for over"), "{out}");
    let one = render_who(&[peer("a", 999, None), peer("b", 1, None)], 1000, false);
    assert!(
        one.contains("1 other not seen for over 10 minutes"),
        "{one}"
    );
}

#[test]
fn who_text_cannot_forge_a_line() {
    let out = render_who(&[peer("x\nforged line", 999, Some("a\nb"))], 1000, true);
    assert_eq!(out.lines().count(), 1, "{out}");
}

#[test]
fn now_reads_ages_and_never_goes_negative() {
    let now = Now(100);
    assert_eq!(now.since(40), 60);
    assert_eq!(now.since(100), 0);
    assert_eq!(
        now.since(160),
        0,
        "a reader clock behind the writer is not a negative age"
    );
}

#[test]
fn who_line_shows_repo_and_dir_and_flattens_them() {
    let mut m = peer("alice", 1000, None);
    m.place = Some(MemberPlace {
        repository: Some("git:proj-1".into()),
        directory: Some("pro\nj".into()),
        scope_differs: false,
    });
    let shown = render_who(&[m], 1000, true);
    assert_eq!(shown.lines().count(), 1, "{shown:?}");
    assert!(shown.contains("repo: git:proj-1\tdir: pro j"), "{shown:?}");
}

#[test]
fn the_short_directory_name_is_the_last_component_and_nothing_more() {
    assert_eq!(short_dir_name("/a/b/proj"), Some("proj".into()));
    assert_eq!(short_dir_name("/a/b/proj/"), Some("proj".into()));
    assert_eq!(short_dir_name("proj"), Some("proj".into()));
    assert_eq!(short_dir_name("/"), None);
    assert_eq!(short_dir_name(""), None);
    assert_eq!(
        short_dir_name(&format!("/x/{}", "d".repeat(100)))
            .unwrap()
            .len(),
        40
    );
}

#[test]
fn a_status_shows_how_long_ago_it_was_set_on_who_and_the_board() {
    let who = render_who(&[aged("a", "done", 1000 - 720)], 1000, false);
    assert!(who.contains("status: done (12m ago)"), "{who}");
    let mut b = sample();
    b.members[1].status_ts = 100 - 45;
    assert!(
        render_board(&b, 100).contains("api-1 last seen 60s ago, status: done leaving (45s ago)")
    );
}

#[test]
fn a_status_of_unknown_age_shows_no_age() {
    let who = render_who(&[aged("a", "done", 0)], 1000, false);
    assert!(who.contains("status: done"), "{who}");
    assert!(!who.contains("ago)"), "{who}");
}

#[test]
fn the_age_is_in_the_largest_whole_unit() {
    assert_eq!(age_phrase(59), "59s");
    assert_eq!(age_phrase(60), "1m");
    assert_eq!(age_phrase(3 * 3600 + 59), "3h");
    assert_eq!(age_phrase(2 * 86_400 + 5), "2d");
    assert_eq!(age_phrase(-5), "0s");
}

#[test]
fn who_shows_the_mode_and_the_wake_path_and_says_unknown_when_no_hook_reported() {
    let text = render_who(
        &[
            with_mode("byp", Some("bypassPermissions")),
            with_mode("zed", Some("default")),
            with_mode("new", None),
        ],
        1000,
        true,
    );
    let line = |n: &str| text.lines().find(|l| l.starts_with(n)).unwrap().to_string();
    assert!(
        line("byp").contains("\tmode: bypassPermissions (wake: hook)"),
        "{text}"
    );
    assert!(
        line("zed").contains("\tmode: default (wake: nudge)"),
        "{text}"
    );
    assert!(
        line("new").contains("\tmode: unknown (wake: hook)"),
        "{text}"
    );
}

#[test]
fn a_mode_with_control_characters_cannot_break_the_who_line() {
    let text = render_who(&[with_mode("x", Some("a\tb\nc"))], 1000, true);
    assert_eq!(text.lines().count(), 1, "{text:?}");
}

#[test]
fn who_marks_an_advisory_member_and_leaves_others_unmarked() {
    let mut flagged = peer("flagged", 990, None);
    flagged.advisory = true;
    let out = render_who(&[flagged, peer("plain", 990, None)], 1000, false);
    let line = |name: &str| {
        out.lines()
            .find(|l| l.starts_with(name))
            .unwrap()
            .to_string()
    };
    assert!(
        line("flagged").contains("\tadvisory: harness lacks a capability this room asked for"),
        "{out}"
    );
    assert!(!line("plain").contains("advisory"), "{out}");
}

#[test]
fn who_marks_a_declared_scope_that_differs_from_the_derived_repository() {
    // #894: the declared scope wins; the mismatch is only made visible, as one fixed word and never a path
    let mut m = peer("alice", 1000, None);
    m.place = Some(MemberPlace {
        repository: Some("git:proj-1".into()),
        directory: Some("proj".into()),
        scope_differs: true,
    });
    let shown = render_who(&[m], 1000, true);
    assert!(shown.contains("\tscope!=repo"), "{shown:?}");
    let mut same = peer("bob", 1000, None);
    same.place = Some(MemberPlace {
        repository: Some("git:proj-1".into()),
        directory: Some("proj".into()),
        scope_differs: false,
    });
    assert!(!render_who(&[same], 1000, true).contains("scope!=repo"));
}
