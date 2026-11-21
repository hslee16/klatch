use super::*;

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
