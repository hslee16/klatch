use super::*;

#[test]
fn nesting_is_at_most_64_levels_in_each_of_state_instructions_and_criteria() {
    assert!(build(&nest(64), &[noul()]).is_ok());
    assert_eq!(
        build(&nest(65), &[noul()]).err(),
        Some(RequestError::TooDeep)
    );
    let instr = |d| Question::noul_with("q", Some(nest(d)), None);
    assert!(build(&text("x"), &[instr(64)]).is_ok());
    assert_eq!(
        build(&text("x"), &[instr(65)]).err(),
        Some(RequestError::TooDeep)
    );
    // criteria: the options object is level 1, so a value may nest 63 more
    let crit = |d| Question::choice_with("q", None, vec![(Key::Fixed("a"), nest(d))]);
    assert!(build(&text("x"), &[crit(63)]).is_ok());
    assert_eq!(
        build(&text("x"), &[crit(64)]).err(),
        Some(RequestError::TooDeep)
    );
    let score = |d| Question::score_with("q", None, vec![nest(d), Entry::fixed("b")]);
    assert!(build(&text("x"), &[score(63)]).is_ok());
    assert_eq!(
        build(&text("x"), &[score(64)]).err(),
        Some(RequestError::TooDeep)
    );
    // the depth of one does not count against another
    assert!(build(&nest(64), &[instr(64)]).is_ok());
}

#[test]
fn from_value_labels_every_leaf_and_refuses_deep_values() {
    let v = json!({"a": ["x", 2, true, null], "b": {"c": "d"}});
    let e = Entry::from_value("public", &v).unwrap();
    assert_eq!(body(&build(&e, &[noul()]).unwrap())["state"], v);
    assert_eq!(
        build(&Entry::from_value("secret", &v).unwrap(), &[noul()]).err(),
        Some(RequestError::Gated)
    );
    let mut deep = json!("x");
    for _ in 0..64 {
        deep = json!([deep]);
    }
    assert!(Entry::from_value("public", &deep).is_ok());
    assert_eq!(
        Entry::from_value("public", &json!([deep])).err(),
        Some(RequestError::TooDeep)
    );
}

#[test]
fn an_integer_too_large_to_send_exactly_is_refused_not_rounded() {
    // api__errors.md: send exact large quantities as strings; an integer that does not fit i64 would lose digits as a float
    let big = json!(u64::MAX);
    assert!(matches!(
        Entry::from_value("public", &big),
        Err(RequestError::BadStructure(_))
    ));
    let e = Entry::from_value("public", &json!([i64::MAX, i64::MIN, 2.5])).unwrap();
    assert_eq!(
        body(&build(&e, &[noul()]).unwrap())["state"],
        json!([i64::MAX, i64::MIN, 2.5])
    );
}

#[test]
fn only_finite_numbers_are_sent() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            build(&Entry::number("public", bad), &[noul()]).err(),
            Some(RequestError::NotFinite)
        );
    }
    assert!(build(&Entry::number("public", -1.5e300), &[noul()]).is_ok());
}

#[test]
fn the_text_sent_is_bounded_by_the_configured_characters() {
    let lim = Limits::new(100, 4, 8).unwrap();
    let go = |n: usize| build_request(MODEL, &text(&"a".repeat(n)), &[noul()], &gate(), &lim);
    // the fixed question wording and keys count too, so find the edge by search and check it is sharp
    let mut ok = 0;
    for n in 0..200 {
        if go(n).is_ok() {
            ok = n;
        }
    }
    assert!(ok > 50 && ok < 100, "{ok}");
    assert!(go(ok).is_ok() && go(ok + 1).err() == Some(RequestError::TooManyChars));
    // characters, not bytes
    let wide = "\u{e9}".repeat(ok);
    assert!(build_request(MODEL, &text(&wide), &[noul()], &gate(), &lim).is_ok());
}

#[test]
fn the_default_bound_is_well_below_what_the_live_check_saw_refused() {
    let l = Limits::default();
    assert_eq!(l.max_text_chars(), DEFAULT_MAX_TEXT_CHARS);
    const { assert!(DEFAULT_MAX_TEXT_CHARS <= 40_000 && DEFAULT_MAX_TEXT_CHARS > 0) };
    const { assert!(HARD_MAX_TEXT_CHARS <= 100_000) };
    const { assert!(HARD_MAX_TEXT_CHARS < 500_000 / 4) };
    assert!(Limits::new(0, 4, 8).is_err());
    assert!(Limits::new(HARD_MAX_TEXT_CHARS, 4, 8).is_ok());
    assert!(Limits::new(HARD_MAX_TEXT_CHARS + 1, 4, 8).is_err());
    assert!(
        Limits::new(10, 0, 8).is_ok(),
        "no images is a valid setting"
    );
    assert!(
        Limits::new(10, 4, 3).is_err(),
        "fewer uses than images makes no sense"
    );
    // the limit is enforced for the real default on 100,000 characters of text
    let r = build(&text(&"x".repeat(100_000)), &[noul()]);
    assert_eq!(r.err(), Some(RequestError::TooManyChars));
}

#[test]
fn the_body_and_expanded_size_checks_are_the_documented_32_mib() {
    assert_eq!(MAX_BODY_BYTES, 32 * 1024 * 1024);
    assert!(check_sizes(1000, 0, MAX_BODY_BYTES).is_ok());
    assert_eq!(
        check_sizes(1000, 0, MAX_BODY_BYTES + 1),
        Err(RequestError::TooLarge)
    );
    // each use adds the image again, and processing can enlarge it (counted twice)
    assert!(check_sizes(0, 0, 10).is_ok());
    assert_eq!(
        check_sizes(0, MAX_BODY_BYTES / 2 + 1, 10),
        Err(RequestError::TooLarge)
    );
    assert!(check_sizes(0, MAX_BODY_BYTES / 2, 10).is_ok());
}
