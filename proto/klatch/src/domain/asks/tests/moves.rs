use super::*;

#[test]
fn a_moved_word_names_its_ask_and_a_valid_member() {
    for (text, expected) in [
        ("moved #3: dm with alpha", Some((3, "alpha"))),
        ("moved #12: dm with a.b-c_d", Some((12, "a.b-c_d"))),
        ("moved #3: dm with alpha.", Some((3, "alpha"))),
        ("moved #3: dm with alpha and more words", Some((3, "alpha"))),
        ("moved #3: dm with", None),
        ("moved #3: dm with ", None),
        ("moved #3: dm with a/b", None),
        ("moved #3: dm with @alpha", None),
        ("moved #3 dm with alpha", None),
        ("moved #: dm with alpha", None),
        ("moved #x: dm with alpha", None),
        ("moved #3: room with alpha", None),
        ("please moved #3: dm with alpha", None),
        ("Moved #3: dm with alpha", None),
    ] {
        let want = expected.map(|(n, w)| (n, w.to_string()));
        assert_eq!(moved_target(text), want, "{text:?}");
    }
}

fn moved_asks(posts: &[(&str, &str)]) -> Vec<OpenAsk> {
    let mut m = vec![msg(1, "asker", "seeking: x", 5)];
    for (i, (who, body)) in posts.iter().enumerate() {
        m.push(msg(i as i64 + 2, who, body, 6));
    }
    open_asks(&m, &[])
}

fn moved_of(posts: &[(&str, &str)]) -> Option<(String, String)> {
    moved_asks(posts)[0]
        .moved
        .as_ref()
        .map(|m| (m.by.clone(), m.with.clone()))
}

#[test]
fn the_asker_or_a_helper_pointing_at_the_asker_moves_an_ask() {
    let want = Some(("asker".to_string(), "helper".to_string()));
    assert_eq!(moved_of(&[("asker", "moved #1: dm with helper")]), want);
    let want = Some(("helper".to_string(), "asker".to_string()));
    assert_eq!(moved_of(&[("helper", "moved #1: dm with asker")]), want);
}

#[test]
fn a_moved_word_that_does_not_involve_the_asker_is_ignored() {
    assert_eq!(moved_of(&[("other", "moved #1: dm with helper")]), None);
    assert_eq!(moved_of(&[("helper", "moved #1: dm with helper")]), None);
    assert_eq!(moved_of(&[("asker", "moved #1: dm with asker")]), None);
    assert_eq!(moved_of(&[("helper", "moved #2: dm with asker")]), None);
    assert_eq!(moved_of(&[("asker", "moved #1: dm with a/b")]), None);
}

#[test]
fn a_moved_word_posted_before_the_ask_is_ignored() {
    let m = [
        msg(1, "asker", "moved #2: dm with helper", 5),
        msg(2, "asker", "seeking: x", 6),
    ];
    assert!(open_asks(&m, &[])[0].moved.is_none());
}

#[test]
fn the_latest_moved_word_wins_and_a_close_still_ends_the_ask() {
    let got = moved_of(&[
        ("asker", "moved #1: dm with one"),
        ("asker", "moved #1: dm with two"),
    ]);
    assert_eq!(got, Some(("asker".to_string(), "two".to_string())));
    let m = [
        msg(1, "asker", "seeking: x", 5),
        msg(2, "asker", "moved #1: dm with helper", 6),
        msg(3, "asker", "closed #1 ok", 7),
    ];
    assert!(open_asks(&m, &[]).is_empty());
}

fn with_dm(last_activity: i64) -> OpenAsk {
    OpenAsk {
        seq: 1,
        sender: "asker".into(),
        moved: Some(MovedDm {
            by: "asker".into(),
            with: "helper".into(),
            room: Some(AskRoom {
                name: "dm.asker.helper".into(),
                members: 2,
                last_activity,
            }),
        }),
        ..OpenAsk::default()
    }
}

#[test]
fn a_dm_with_news_keeps_a_moved_ask_alive_and_a_quiet_one_does_not() {
    assert!(with_dm(100).room_active(150));
    assert!(!with_dm(100).room_active(100 + ROOM_ACTIVE_SECS + 1));
    let mut no_room = with_dm(100);
    no_room.moved.as_mut().unwrap().room = None;
    assert!(!no_room.room_active(100));
    assert!(!OpenAsk::default().room_active(100));
}

#[test]
fn a_declared_scope_leads_the_summary_and_no_scope_adds_nothing() {
    let mut a = OpenAsk::default();
    assert_eq!(a.summary(0), "");
    a.scope = Some("git:foo".into());
    assert_eq!(a.summary(0), "[scope git:foo]");
    a.outcomes.push(Outcome {
        kind: OutcomeKind::Busy,
        by: "bob".into(),
        reason: String::new(),
    });
    assert_eq!(a.summary(0), "[scope git:foo, busy 1]");
}

#[test]
fn a_moved_ask_is_shown_as_in_a_dm_with_the_name_only() {
    let a = with_dm(100);
    assert_eq!(a.summary(150), "[in a dm with helper]");
    assert!(!a.summary(150).contains("dm.asker"));
    let mut b = a.clone();
    b.waiting_on_asker = Some(Clarification {
        by: "helper".into(),
        question: "SECRET TEXT".into(),
    });
    assert!(b.summary(150).contains("in a dm with helper"));
    assert!(!b.summary(150).contains("SECRET"));
    let mut unseen = a;
    unseen.moved.as_mut().unwrap().room = None;
    assert_eq!(unseen.summary(150), "");
}

#[test]
fn only_the_asker_can_reopen() {
    let m = [
        msg(1, "asker", "seeking: slugify", 5),
        msg(2, "helper", "answer to #1: ok", 6),
        msg(3, "other", "reopen #1: not yours", 7),
    ];
    assert!(open_asks(&m, &[]).is_empty());
}

#[test]
fn a_reopen_before_the_answer_changes_nothing() {
    let m = [
        msg(1, "asker", "seeking: slugify", 5),
        msg(2, "asker", "reopen #1: still open", 6),
        msg(3, "helper", "answer to #1: ok", 7),
    ];
    assert!(open_asks(&m, &[]).is_empty());
}

#[test]
fn a_second_answer_ends_a_reopened_ask_again() {
    let m = [
        msg(1, "asker", "seeking: slugify", 5),
        msg(2, "h1", "answer to #1: a", 6),
        msg(3, "asker", "reopen #1: no", 7),
        msg(4, "h2", "answer to #1: b", 8),
    ];
    assert!(open_asks(&m, &[]).is_empty());
    let mut m = m.to_vec();
    m.push(msg(5, "asker", "reopen #1: still no", 9));
    assert_eq!(open_asks(&m, &[]).len(), 1);
}

#[test]
fn a_reopened_ask_keeps_what_helpers_said_before() {
    let m = [
        msg(1, "asker", "seeking: slugify", 5),
        msg(2, "h1", "failed #1: no luck", 6),
        msg(3, "h1", "answer to #1: partial fix", 7),
        msg(4, "asker", "reopen #1: not enough", 8),
    ];
    let o = open_asks(&m, &[]);
    assert_eq!(o[0].outcomes.len(), 1);
}
