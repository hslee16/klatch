use super::*;

/// The label in every place of the state.
fn labelled_in_state(label: &str) -> Vec<(&'static str, Entry, Question)> {
    let t = || Entry::text(label, "secret-thing");
    vec![
        ("state text", t(), noul()),
        ("state number", Entry::number(label, 1.0), noul()),
        ("state flag", Entry::flag(label, true), noul()),
        (
            "state key",
            Entry::object(vec![(
                Key::Data {
                    label: label.into(),
                    name: "secret-thing".into(),
                },
                Entry::Null,
            )]),
            noul(),
        ),
        ("state image", Entry::image(label, png(1)), noul()),
        (
            "state nested",
            Entry::list(vec![Entry::object(vec![(Key::Fixed("k"), t())])]),
            noul(),
        ),
        (
            "sentence",
            Entry::sentence(vec![Entry::fixed("see "), t()]),
            noul(),
        ),
    ]
}

/// The label in every place of a question.
fn labelled_in_question(label: &str) -> Vec<(&'static str, Entry, Question)> {
    let t = || Entry::text(label, "secret-thing");
    vec![
        (
            "instructions",
            text("x"),
            Question::noul_with("q", Some(t()), None),
        ),
        (
            "noul criteria",
            text("x"),
            Question::noul_with(
                "q",
                None,
                Some(NoulCriteria {
                    when_true: Some(t()),
                    when_false: None,
                }),
            ),
        ),
        (
            "choice value",
            text("x"),
            Question::choice_with("q", None, vec![(Key::Fixed("a"), t())]),
        ),
        (
            "choice key",
            text("x"),
            Question::choice_with(
                "q",
                None,
                vec![(
                    Key::Data {
                        label: label.into(),
                        name: "secret-thing".into(),
                    },
                    Entry::fixed("d"),
                )],
            ),
        ),
        (
            "score level",
            text("x"),
            Question::score_with("q", None, vec![Entry::fixed("a"), t()]),
        ),
    ]
}

/// One labelled thing in every place a label can sit, as a state and a question that carry it.
fn labelled_everywhere(label: &str) -> Vec<(&'static str, Entry, Question)> {
    let mut all = labelled_in_state(label);
    all.extend(labelled_in_question(label));
    all
}

#[test]
fn every_labelled_thing_is_gated_wherever_it_sits() {
    let ok = |st: Entry, q: Question| build(&st, &[q]).is_ok();
    for label in ["internal", "private", "secret", "", "unknown-label"] {
        for (what, st, q) in labelled_everywhere(label) {
            assert_eq!(
                build(&st, &[q]).err(),
                Some(RequestError::Gated),
                "{what} at {label:?}"
            );
        }
    }
    for (what, st, q) in labelled_everywhere("public") {
        assert!(ok(st, q), "{what} below the ceiling is sent");
    }
}

#[test]
fn a_closed_gate_sends_labelled_data_never_and_fixed_wording_always() {
    let closed = Gate::closed();
    let r = build_request(MODEL, &text("x"), &[noul()], &closed, &limits());
    assert_eq!(r.err(), Some(RequestError::Gated));
    assert!(build_request(MODEL, &Entry::fixed("x"), &[noul()], &closed, &limits()).is_ok());
    let no_ceiling = Gate::new(LabelChain::default(), None);
    assert!(build_request(MODEL, &text("x"), &[noul()], &no_ceiling, &limits()).is_err());
}

#[test]
fn the_message_never_appears_in_a_refusal() {
    let e = build(&Entry::text("secret", "TOPSECRET-TEXT"), &[noul()]).unwrap_err();
    assert!(!format!("{e:?} {e}").contains("TOPSECRET"));
    let dbg = format!("{:?}", Entry::text("public", "TOPSECRET-TEXT"));
    assert!(!dbg.contains("TOPSECRET"), "{dbg}");
}
