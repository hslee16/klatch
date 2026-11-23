use super::*;

fn with(kind: OutcomeKind, ask: i64, who: &str) -> Message {
    let word = match kind {
        OutcomeKind::Busy => format!("declined #{ask} busy: r"),
        OutcomeKind::Cannot => format!("declined #{ask} cannot: r"),
        OutcomeKind::Failed => format!("failed #{ask}: r"),
        OutcomeKind::Partial => format!("partial #{ask}: r"),
    };
    msg(10, who, &word, 9)
}

#[test]
fn an_open_ask_carries_the_outcomes_other_helpers_gave_it() {
    let m = [
        msg(1, "asker", "seeking: x", 5),
        with(OutcomeKind::Busy, 1, "h1"),
        with(OutcomeKind::Cannot, 1, "h2"),
        with(OutcomeKind::Cannot, 2, "h3"),
        with(OutcomeKind::Failed, 1, "asker"),
    ];
    let o = open_asks(&m, &[]);
    assert_eq!(o[0].outcomes.len(), 2, "only other helpers, only this ask");
    assert_eq!(
        (o[0].outcomes[0].by.as_str(), o[0].outcomes[0].kind),
        ("h1", OutcomeKind::Busy)
    );
}

#[test]
fn nobody_here_can_when_two_distinct_helpers_cannot_and_none_is_busy_or_failed() {
    let ask = |outs: &[(OutcomeKind, &str)]| {
        let mut m = vec![msg(1, "asker", "seeking: x", 5)];
        for (k, w) in outs {
            m.push(with(*k, 1, w));
        }
        open_asks(&m, &[]).remove(0)
    };
    let two = ask(&[(OutcomeKind::Cannot, "a"), (OutcomeKind::Cannot, "b")]);
    assert_eq!(two.distinct(OutcomeKind::Cannot), 2);
    assert!(two.no_capable_member());
    assert!(!ask(&[(OutcomeKind::Cannot, "a")]).no_capable_member());
    assert!(
        !ask(&[(OutcomeKind::Cannot, "a"), (OutcomeKind::Cannot, "a")]).no_capable_member(),
        "the same helper twice is one"
    );
    assert!(
        !ask(&[
            (OutcomeKind::Cannot, "a"),
            (OutcomeKind::Cannot, "b"),
            (OutcomeKind::Busy, "c")
        ])
        .no_capable_member()
    );
    assert!(
        !ask(&[
            (OutcomeKind::Cannot, "a"),
            (OutcomeKind::Cannot, "b"),
            (OutcomeKind::Failed, "c")
        ])
        .no_capable_member()
    );
}

#[test]
fn the_lease_of_an_ask_claim_is_short_by_default_and_capped() {
    let cases = [
        (None, 90),
        (Some(30), 30),
        (Some(3600), 300), // an ask lease is capped
    ];
    for (asked, expected) in cases {
        assert_eq!(claim_ttl("ask:7", asked), expected, "{asked:?}");
    }
}

#[test]
fn other_claims_keep_their_default_and_are_not_capped_here() {
    assert_eq!(claim_ttl("src/lib.rs", None), 600);
    assert_eq!(claim_ttl("src/lib.rs", Some(3600)), 3600);
}

#[test]
fn a_partial_note_is_parsed_and_shown_and_counts_as_someone_capable() {
    assert_eq!(
        parse_outcome("partial #4: parsed the file, tests not written"),
        Some((
            OutcomeKind::Partial,
            4,
            "parsed the file, tests not written".into()
        ))
    );
    assert_eq!(parse_outcome("partial #4 no colon"), None);
    let mut m = vec![msg(1, "asker", "seeking: x", 5)];
    for (i, who) in ["h1", "h2"].iter().enumerate() {
        m.push(msg(10 + i as i64, who, "declined #1 cannot: no", 6));
    }
    m.push(msg(20, "h3", "partial #1: half done", 7));
    let a = open_asks(&m, &[]).remove(0);
    assert!(a.summary(10).contains("partial 1"), "{}", a.summary(10));
    assert!(
        !a.no_capable_member(),
        "someone worked on it, so someone here can"
    );
}

#[test]
fn clarification_words_are_parsed_with_their_kind_ask_and_text() {
    let cases = [
        (
            "need-info #3: which format?",
            Some((true, 3, "which format?")),
        ),
        ("info #3: json", Some((false, 3, "json"))),
        ("info #3 json", None),
        ("need-info #x: q", None),
        ("my info #3: q", None),
    ];
    for (text, expected) in cases {
        let expected = expected.map(|(need, n, t)| (need, n, t.to_string()));
        assert_eq!(parse_clarification(text), expected, "{text:?}");
    }
}

#[test]
fn an_ask_waits_on_its_asker_until_the_asker_answers_the_helpers_question() {
    let base = vec![
        msg(1, "asker", "seeking: x", 5),
        msg(2, "helper", "need-info #1: which format?", 6),
    ];
    let waiting = open_asks(&base, &[]).remove(0);
    let w = waiting.waiting_on_asker.expect("waiting on the asker");
    assert_eq!(
        (w.by.as_str(), w.question.as_str()),
        ("helper", "which format?")
    );
    let mut answered = base.clone();
    answered.push(msg(3, "asker", "info #1: json", 7));
    assert!(
        open_asks(&answered, &[])
            .remove(0)
            .waiting_on_asker
            .is_none()
    );
    let mut again = answered.clone();
    again.push(msg(4, "helper", "need-info #1: and the encoding?", 8));
    assert_eq!(
        open_asks(&again, &[])
            .remove(0)
            .waiting_on_asker
            .unwrap()
            .question,
        "and the encoding?"
    );
}

#[test]
fn only_a_helper_can_ask_and_only_the_asker_can_answer() {
    let m = vec![
        msg(1, "asker", "seeking: x", 5),
        msg(2, "asker", "need-info #1: asking myself", 6),
        msg(3, "stranger", "info #1: not mine to say", 7),
    ];
    assert!(
        open_asks(&m, &[]).remove(0).waiting_on_asker.is_none(),
        "the asker cannot wait on itself"
    );
    let m2 = vec![
        msg(1, "asker", "seeking: x", 5),
        msg(2, "helper", "need-info #1: q", 6),
        msg(3, "stranger", "info #1: not the asker", 7),
    ];
    assert!(
        open_asks(&m2, &[]).remove(0).waiting_on_asker.is_some(),
        "a stranger's info does not answer it"
    );
}

#[test]
fn the_summary_names_the_wait_and_the_working_room_and_the_room_counts_as_activity() {
    let mut a = open_asks(&[msg(7, "asker", "seeking: x", 5)], &[]).remove(0);
    assert_eq!(a.summary(100), "");
    a.waiting_on_asker = Some(Clarification {
        by: "h".into(),
        question: "q".into(),
    });
    a.room = Some(AskRoom {
        name: "ask-7".into(),
        members: 3,
        last_activity: 90,
    });
    let s = a.summary(100);
    assert!(
        s.contains("needs info") && s.contains("room ask-7 3 members"),
        "{s}"
    );
    assert!(
        a.room_active(100) && a.room_active(210),
        "news within two minutes"
    );
    assert!(!a.room_active(211), "a quiet room is not activity");
}
