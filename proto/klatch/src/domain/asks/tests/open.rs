use super::*;

#[test]
fn an_unanswered_ask_is_open_and_unclaimed() {
    let m = [msg(1, "asker", "seeking: slugify", 5)];
    let o = open_asks(&m, &[]);
    assert_eq!(o.len(), 1);
    assert_eq!(
        (o[0].seq, o[0].sender.as_str(), o[0].ts, o[0].claimed),
        (1, "asker", 5, false)
    );
    assert_eq!(o[0].text, "seeking: slugify");
}

#[test]
fn a_live_claim_on_ask_n_marks_it_claimed() {
    let m = [msg(4, "asker", "seeking: x", 5)];
    assert!(open_asks(&m, &[claim("ask:4")])[0].claimed);
    assert!(!open_asks(&m, &[claim("ask:5")])[0].claimed);
    assert!(!open_asks(&m, &[claim("src")])[0].claimed);
}

#[test]
fn a_reply_from_someone_else_ends_the_ask() {
    let m = [
        msg(1, "asker", "seeking: x", 5),
        msg(2, "helper", "answer to #1: here", 9),
    ];
    assert!(open_asks(&m, &[]).is_empty());
}

#[test]
fn the_askers_own_answer_to_itself_does_not_end_the_ask() {
    let m = [
        msg(1, "asker", "seeking: x", 5),
        msg(2, "asker", "answer to #1: me", 9),
    ];
    assert_eq!(open_asks(&m, &[]).len(), 1);
}

#[test]
fn the_asker_closing_ends_the_ask_but_someone_elses_close_does_not() {
    let closed = [
        msg(1, "asker", "seeking: x", 5),
        msg(2, "asker", "closed #1", 9),
    ];
    assert!(open_asks(&closed, &[]).is_empty());
    let other = [
        msg(1, "asker", "seeking: x", 5),
        msg(2, "stranger", "closed #1", 9),
    ];
    assert_eq!(open_asks(&other, &[]).len(), 1);
}

#[test]
fn a_reply_only_ends_the_ask_it_names() {
    let m = [
        msg(1, "a", "seeking: one", 1),
        msg(2, "b", "seeking: two", 2),
        msg(3, "h", "answer to #2: ok", 3),
    ];
    let o = open_asks(&m, &[]);
    assert_eq!(o.len(), 1);
    assert_eq!(o[0].seq, 1);
}

#[test]
fn non_asks_are_ignored_and_order_is_by_seq() {
    let m = [
        msg(1, "a", "hello", 1),
        msg(2, "a", "seeking: one", 2),
        msg(3, "b", "chatter", 3),
        msg(4, "b", "seeking: two", 4),
    ];
    let seqs: Vec<_> = open_asks(&m, &[]).iter().map(|a| a.seq).collect();
    assert_eq!(seqs, [2, 4]);
}

#[test]
fn outcomes_are_parsed_with_their_kind_ask_and_reason() {
    let cases = [
        (
            "declined #4 busy: on LIB-2",
            Some((OutcomeKind::Busy, 4, "on LIB-2")),
        ),
        (
            "declined #12 cannot: not my area",
            Some((OutcomeKind::Cannot, 12, "not my area")),
        ),
        (
            "failed #3: the build broke",
            Some((OutcomeKind::Failed, 3, "the build broke")),
        ),
        ("declined #4 busy:", Some((OutcomeKind::Busy, 4, ""))),
    ];
    for (text, expected) in cases {
        let expected = expected.map(|(k, n, r)| (k, n, r.to_string()));
        assert_eq!(parse_outcome(text), expected, "{text:?}");
    }
}

#[test]
fn a_malformed_outcome_is_not_parsed() {
    let cases = [
        "declined #4: no class",
        "declined #x busy: r",
        "failed #3 the build broke",
        "I declined #4 busy: r",
    ];
    for text in cases {
        assert_eq!(parse_outcome(text), None, "{text:?}");
    }
}
