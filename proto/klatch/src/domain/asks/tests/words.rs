use super::*;

#[test]
fn an_ask_starts_with_the_prefix() {
    let cases = [
        ("seeking: a thing", true),
        ("seeking:no space", true),
        ("not seeking: a thing", false),
        ("Seeking: a thing", false),
        (" seeking: leading space", false),
        ("", false),
    ];
    for (text, expected) in cases {
        assert_eq!(is_ask(text), expected, "{text:?}");
    }
}

/// A word that names its ask as `WORD #N: text`: the colon and the number are both required and the word starts the text.
fn check_colon_word(word: &str, target: fn(&str) -> Option<i64>) {
    let cases = [
        (format!("{word} #3: here"), Some(3)),
        (format!("{word} #12:x"), Some(12)),
        (format!("{word} #3 no colon"), None),
        (format!("{word} #: x"), None),
        (format!("my {word} #3: x"), None),
    ];
    for (text, expected) in cases {
        assert_eq!(target(&text), expected, "{text:?}");
    }
}

#[test]
fn answers_name_their_ask_by_number() {
    check_colon_word("answer to", answer_target);
}

#[test]
fn closes_name_their_ask_by_number() {
    let cases = [
        ("closed #3", Some(3)),
        ("closed #3 thanks", Some(3)),
        ("closed #3.", Some(3)),
        ("closed #x", None),
        ("not closed #3", None),
        ("closed #", None),
    ];
    for (text, expected) in cases {
        assert_eq!(closed_target(text), expected, "{text:?}");
    }
}

#[test]
fn a_close_names_its_outcome() {
    for (text, expected) in [
        ("closed #3", Some(CloseOutcome::Unspecified)),
        ("closed #3 thanks", Some(CloseOutcome::Unspecified)),
        ("closed #3 ok", Some(CloseOutcome::Ok)),
        ("closed #3 OK.", Some(CloseOutcome::Ok)),
        ("closed #3 ok thanks", Some(CloseOutcome::Ok)),
        ("closed #3 unresolved", Some(CloseOutcome::Unresolved)),
        (
            "closed #3 unresolved: nobody knew",
            Some(CloseOutcome::Unresolved),
        ),
        ("closed #3 okay", Some(CloseOutcome::Unspecified)),
        ("closed #3ok", Some(CloseOutcome::Unspecified)),
        ("closed #x ok", None),
        ("not closed #3 ok", None),
    ] {
        assert_eq!(close_outcome(text), expected, "{text:?}");
    }
}

#[test]
fn every_close_outcome_still_ends_the_ask() {
    for body in ["closed #1", "closed #1 ok", "closed #1 unresolved"] {
        let m = [msg(1, "asker", "seeking: x", 5), msg(2, "asker", body, 6)];
        assert!(open_asks(&m, &[]).is_empty(), "{body}");
    }
}

#[test]
fn latest_close_is_the_askers_newest_close_after_any_reopen() {
    let ask = msg(1, "asker", "seeking: x", 5);
    let mut m = vec![ask.clone(), msg(2, "asker", "closed #1 unresolved", 6)];
    assert_eq!(latest_close(&m, &ask), Some(CloseOutcome::Unresolved));
    m.push(msg(3, "other", "closed #1 ok", 7));
    assert_eq!(latest_close(&m, &ask), Some(CloseOutcome::Unresolved));
    m.push(msg(4, "asker", "reopen #1: try again", 8));
    assert_eq!(latest_close(&m, &ask), None);
    m.push(msg(5, "asker", "closed #1 ok", 9));
    assert_eq!(latest_close(&m, &ask), Some(CloseOutcome::Ok));
}

#[test]
fn reopens_name_their_ask_by_number() {
    check_colon_word("reopen", reopen_target);
}

#[test]
fn the_asker_reopens_an_answered_ask() {
    let m = [
        msg(1, "asker", "seeking: slugify", 5),
        msg(2, "helper", "answer to #1: wrong", 6),
        msg(3, "asker", "reopen #1: that is the wrong crate", 7),
    ];
    let o = open_asks(&m, &[]);
    assert_eq!(o.len(), 1);
    assert_eq!((o[0].seq, o[0].claimed), (1, false));
}

#[test]
fn the_asker_reopens_a_closed_ask() {
    let m = [
        msg(1, "asker", "seeking: slugify", 5),
        msg(2, "asker", "closed #1", 6),
        msg(3, "asker", "reopen #1: closed too soon", 7),
    ];
    assert_eq!(open_asks(&m, &[]).len(), 1);
}
