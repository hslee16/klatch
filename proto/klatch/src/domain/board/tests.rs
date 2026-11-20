use super::*;
use crate::domain::Attribution;

/// A message with this seq, (sender, body) and timestamp.
fn msg(seq: i64, said: (&str, &str), ts: i64) -> Message {
    let (sender, body) = said;
    Message {
        id: format!("id{seq}"),
        seq,
        sender: sender.into(),
        attribution: Attribution::Unattributed,
        label: "public".into(),
        body: body.into(),
        ts,
        kind: None,
        payload: None,
    }
}

fn peer(name: &str, last_seen: i64, status: Option<&str>) -> MemberInfo {
    MemberInfo {
        status_ts: 0,
        name: name.into(),
        joined: 0,
        last_seen,
        status: status.map(Into::into),
        parent: None,
        parked: None,
        kind: None,
        place: None,
        mode: None,
        advisory: false,
    }
}

#[test]
fn an_ask_with_nobody_else_present_says_so() {
    let note = ask_reach_note(&[peer("asker", 1000, None)], "asker", 1000).unwrap();
    assert!(note.contains("do not wait"));
    assert!(!note.contains('\u{2014}'));
}

#[test]
fn an_ask_with_a_recent_helper_says_nothing() {
    let m = [peer("asker", 1000, None), peer("helper", 900, None)];
    assert_eq!(ask_reach_note(&m, "asker", 1000), None);
}

#[test]
fn a_helper_gone_past_the_window_or_done_does_not_count() {
    let stale = [
        peer("asker", 5000, None),
        peer("helper", 5000 - MEMBER_ACTIVE_SECS - 1, None),
    ];
    assert!(ask_reach_note(&stale, "asker", 5000).is_some());
    let done = [
        peer("asker", 5000, None),
        peer("helper", 4999, Some("done bye")),
    ];
    assert!(ask_reach_note(&done, "asker", 5000).is_some());
}

#[test]
fn a_helper_that_declared_a_wait_still_counts() {
    let mut h = peer("helper", 0, None);
    h.parked = Some(Parked {
        why: "ci".into(),
        back_by: 9000,
    });
    let m = [peer("asker", 5000, None), h];
    assert_eq!(ask_reach_note(&m, "asker", 5000), None);
}

#[test]
fn an_offer_starts_with_the_prefix() {
    assert!(is_offer("offering: slugify help"));
    assert!(!is_offer("Offering: x"));
    assert!(!is_offer(" offering: x"));
    assert!(!is_offer("seeking: x"));
}

#[test]
fn an_offer_is_current_with_its_sender_and_age() {
    let m = [msg(1, ("helper", "offering: text helpers"), 10)];
    let o = current_offers(&m, 20, 3600);
    assert_eq!(o.len(), 1);
    assert_eq!((o[0].sender.as_str(), o[0].seq, o[0].ts), ("helper", 1, 10));
    assert_eq!(o[0].text, "offering: text helpers");
}

#[test]
fn a_new_offer_from_the_same_sender_replaces_the_old_one() {
    let m = [
        msg(1, ("helper", "offering: old"), 10),
        msg(2, ("helper", "offering: new"), 20),
    ];
    let o = current_offers(&m, 30, 3600);
    assert_eq!(o.len(), 1);
    assert_eq!(o[0].text, "offering: new");
}

#[test]
fn a_withdrawal_clears_the_senders_offer_and_only_theirs() {
    let m = [
        msg(1, ("a", "offering: x"), 1),
        msg(2, ("b", "offering: y"), 2),
        msg(3, ("a", "withdrawn: no longer"), 3),
    ];
    let o = current_offers(&m, 10, 3600);
    assert_eq!(o.len(), 1);
    assert_eq!(o[0].sender, "b");
}

#[test]
fn status_done_clears_the_offer_but_another_status_does_not() {
    let done = [
        msg(1, ("a", "offering: x"), 1),
        msg(2, ("a", "status: done leaving"), 2),
    ];
    assert!(current_offers(&done, 10, 3600).is_empty());
    let busy = [
        msg(1, ("a", "offering: x"), 1),
        msg(2, ("a", "status: busy"), 2),
    ];
    assert_eq!(current_offers(&busy, 10, 3600).len(), 1);
    let back = [
        msg(1, ("a", "status: done"), 1),
        msg(2, ("a", "offering: again"), 2),
    ];
    assert_eq!(
        current_offers(&back, 10, 3600).len(),
        1,
        "an offer after done is current"
    );
}

#[test]
fn an_offer_expires_after_the_lifetime() {
    let m = [msg(1, ("a", "offering: x"), 100)];
    assert_eq!(current_offers(&m, 100 + 3600, 3600).len(), 1);
    assert!(current_offers(&m, 100 + 3601, 3600).is_empty());
}

#[test]
fn offers_come_oldest_first_and_other_messages_are_ignored() {
    let m = [
        msg(1, ("a", "hello"), 1),
        msg(2, ("b", "offering: b thing"), 2),
        msg(3, ("c", "seeking: z"), 3),
        msg(4, ("a", "offering: a thing"), 4),
    ];
    let who: Vec<_> = current_offers(&m, 10, 3600)
        .iter()
        .map(|o| o.sender.clone())
        .collect();
    assert_eq!(who, ["b", "a"]);
}

#[test]
fn statuses_are_the_latest_per_member() {
    let m = [
        msg(1, ("a", "status: busy"), 1),
        msg(2, ("b", "status: done"), 2),
        msg(3, ("a", "status: done leaving"), 3),
        msg(4, ("b", "chatter"), 4),
    ];
    let s = statuses(&m);
    assert_eq!(s.len(), 2);
    assert_eq!(
        (s[0].member.as_str(), s[0].status.as_str()),
        ("a", "done leaving")
    );
    assert_eq!((s[1].member.as_str(), s[1].status.as_str()), ("b", "done"));
}

/// An unclaimed ask with this seq, (sender, text) and timestamp.
fn ask(seq: i64, said: (&str, &str), ts: i64) -> OpenAsk {
    let (sender, text) = said;
    OpenAsk {
        seq,
        sender: sender.into(),
        text: text.into(),
        ts,
        ..Default::default()
    }
}

fn claimed(ask: OpenAsk) -> OpenAsk {
    OpenAsk {
        claimed: true,
        ..ask
    }
}

fn sample() -> Board {
    Board {
        room: "lobby".into(),
        asks: vec![
            ask(1, ("api-1", "seeking: slugify"), 90),
            claimed(ask(2, ("docs-1", "seeking: the api"), 80)),
        ],
        offers: vec![Offer {
            sender: "lib-1".into(),
            seq: 3,
            text: "offering: text helpers".into(),
            ts: 50,
        }],
        members: vec![
            BoardMember {
                status_ts: 0,
                name: "lib-1".into(),
                last_seen: 98,
                status: None,
                parent: None,
                parked: None,
                kind: None,
                place: None,
                advisory: false,
            },
            BoardMember {
                status_ts: 0,
                name: "api-1".into(),
                last_seen: 40,
                status: Some("done leaving".into()),
                parent: None,
                parked: None,
                kind: None,
                place: None,
                advisory: false,
            },
        ],
        stale_members: 0,
        tasks: vec![],
        prs: vec![],
    }
}

#[test]
fn the_board_shows_asks_offers_and_members_with_ages_and_status() {
    let out = render_board(&sample(), 100);
    assert!(out.starts_with("[board lobby]"), "got: {out}");
    assert!(
        out.contains("#1 api-1 unclaimed 10s: seeking: slugify"),
        "got: {out}"
    );
    assert!(out.contains("#2 docs-1 claimed 20s: seeking: the api"));
    assert!(out.contains("lib-1 offers (50s ago): offering: text helpers"));
    assert!(out.contains("lib-1 last seen 2s ago"));
    assert!(out.contains("api-1 last seen 60s ago, status: done leaving"));
}

#[test]
fn an_empty_board_names_its_empty_sections() {
    let out = render_board(
        &Board {
            room: "r".into(),
            ..Default::default()
        },
        1,
    );
    assert!(
        out.contains("no open asks") && out.contains("no current offers"),
        "got: {out}"
    );
}

#[test]
fn board_text_cannot_forge_a_header_or_grow_without_limit() {
    let mut b = sample();
    b.offers[0].text = format!("offering: x\n[board evil]\n{}", "y".repeat(500));
    b.asks[0].text = "seeking: a\n[board evil2]".into();
    let out = render_board(&b, 100);
    assert_eq!(
        out.lines().filter(|l| l.starts_with("[board ")).count(),
        1,
        "forged a header: {out}"
    );
    assert!(
        out.lines().all(|l| l.chars().count() < 260),
        "unbounded: {out}"
    );
}

#[test]
fn members_not_seen_for_a_while_are_counted_not_listed() {
    let mut b = sample();
    b.stale_members = 3;
    let out = render_board(&b, 100);
    assert!(
        out.contains("3 others not seen for over 10 minutes"),
        "got: {out}"
    );
    b.stale_members = 1;
    assert!(render_board(&b, 100).contains("1 other not seen for over 10 minutes"));
    b.stale_members = 0;
    assert!(!render_board(&b, 100).contains("not seen for over"));
}

#[test]
fn long_sections_are_cut_and_the_rest_counted() {
    let mut b = sample();
    b.asks = (1..=25).map(|i| ask(i, ("a", "seeking: x"), 90)).collect();
    let out = render_board(&b, 100);
    assert_eq!(
        out.lines()
            .filter(|l| l.trim_start().starts_with('#'))
            .count(),
        20
    );
    assert!(out.contains("and 5 more asks"), "got: {out}");
}

#[test]
fn an_ask_shows_how_helpers_answered_it_without_taking_it() {
    let mut b = sample();
    b.asks[0].outcomes = vec![
        crate::domain::Outcome {
            kind: crate::domain::OutcomeKind::Busy,
            by: "h1".into(),
            reason: "x".into(),
        },
        crate::domain::Outcome {
            kind: crate::domain::OutcomeKind::Cannot,
            by: "h2".into(),
            reason: "y".into(),
        },
        crate::domain::Outcome {
            kind: crate::domain::OutcomeKind::Cannot,
            by: "h3".into(),
            reason: "y".into(),
        },
    ];
    let out = render_board(&b, 100);
    assert!(
        out.contains("#1 api-1 unclaimed 10s [busy 1, cannot 2]: seeking: slugify"),
        "got: {out}"
    );
    assert!(
        out.contains("#2 docs-1 claimed 20s: seeking: the api"),
        "an ask with no outcomes keeps its short form"
    );
}

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
fn open_tasks_show_on_the_board_with_their_checkpoint_and_an_empty_board_has_no_tasks_section() {
    let task = crate::domain::Task {
        owner: "alice".into(),
        title: "nightly build".into(),
        what: "nightly build".into(),
        started: 100,
        last: 200,
        checkpoint: Some(crate::domain::Checkpoint {
            text: "resume at step 4".into(),
            ts: 200,
        }),
        from: None,
        orphaned: true,
    };
    let board = Board {
        room: "lobby".into(),
        tasks: vec![task],
        ..Default::default()
    };
    let out = render_board(&board, 200 + 86_400);
    assert!(
        out.contains("tasks:\n  alice: nightly build (orphaned")
            && out.contains("checkpoint 1d ago: resume at step 4"),
        "{out}"
    );
    let empty = render_board(&Board::default(), 10);
    assert!(!empty.contains("tasks:"), "{empty}");
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

fn aged(name: &str, status: &str, set_at: i64) -> MemberInfo {
    MemberInfo {
        status_ts: set_at,
        ..peer(name, 990, Some(status))
    }
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
fn available_is_not_done_so_it_keeps_the_offer_and_the_reach() {
    let offers = [
        msg(1, ("a", "offering: help"), 1),
        msg(2, ("a", "status: available"), 2),
    ];
    assert_eq!(current_offers(&offers, 10, 3600).len(), 1);
    assert_eq!(statuses(&offers)[0].status, "available");
    let m = [peer("helper", 4999, Some("available"))];
    assert!(ask_reach_note(&m, "asker", 5000).is_none());
}

fn with_mode(name: &str, mode: Option<&str>) -> MemberInfo {
    MemberInfo {
        mode: mode.map(Into::into),
        ..peer(name, 1000, None)
    }
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
fn the_ask_notice_names_a_bypassing_member_that_gets_no_nudge() {
    let members = [
        peer("asker", 1000, None),
        with_mode("byp", Some("bypassPermissions")),
        with_mode("zed", Some("default")),
    ];
    let note = nudge_skipped_note(&members, "asker", 1000).unwrap();
    assert!(note.contains("byp"), "{note}");
    assert!(note.contains("bypassPermissions"), "{note}");
    assert!(!note.contains("zed"), "{note}");
    assert!(!note.contains('\u{2014}'));
}

#[test]
fn no_nudge_notice_for_prompting_unknown_gone_or_own_modes() {
    let members = [
        with_mode("asker", Some("bypassPermissions")),
        with_mode("pro", Some("plan")),
        with_mode("new", None),
        MemberInfo {
            mode: Some("bypassPermissions".into()),
            advisory: false,
            ..peer("gone", 1000 - MEMBER_ACTIVE_SECS - 1, None)
        },
    ];
    assert_eq!(nudge_skipped_note(&members, "asker", 1000), None);
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
