use super::*;
use crate::domain::LabelChain;
use serde_json::{Value, json};

const MODEL: &str = "jev-1.14.0";
const TEXT: &str = "hello, can we schedule the weekly sync for tuesday?";

fn gate() -> Gate {
    Gate::new(LabelChain::default(), Some("internal"))
}

fn limits() -> Limits {
    Limits::default()
}

fn build(state: &Entry, qs: &[Question]) -> Result<Prepared, RequestError> {
    build_request(MODEL, state, qs, &gate(), &limits())
}

fn body(p: &Prepared) -> Value {
    serde_json::from_str(p.body()).unwrap()
}

fn text(s: &str) -> Entry {
    Entry::text("public", s)
}

/// A PNG header of the given size and a unique tail, so two of them differ.
fn png(seed: u8) -> Image {
    let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    v.extend(13u32.to_be_bytes());
    v.extend(b"IHDR");
    v.extend(300u32.to_be_bytes());
    v.extend(300u32.to_be_bytes());
    v.extend([8, 2, 0, 0, 0, 0, 0, 0, 0]);
    v.extend(4u32.to_be_bytes());
    v.extend(b"IDAT");
    v.extend([seed; 4]);
    v.extend([0, 0, 0, 0]);
    Image::from_bytes(v).unwrap()
}

fn noul() -> Question {
    Question::noul("q", "Is it about billing?")
}

fn nest(depth: usize) -> Entry {
    let mut e = text("leaf");
    for _ in 0..depth {
        e = Entry::list(vec![e]);
    }
    e
}

// ---- the body ----

#[test]
fn the_four_kinds_have_the_documented_wire_shapes() {
    let qs = [
        Question::choice("c", "Tone?", &[("angry", "upset"), ("calm", "neutral")]),
        Question::noul("n", "Billing?"),
        Question::score("s", "Urgent?", &["wait", "today"]),
    ];
    let b = body(&build(&text(TEXT), &qs).unwrap());
    assert_eq!(b["model"], MODEL);
    assert_eq!(b["state"], TEXT);
    assert!(b.get("assets").is_none());
    assert_eq!(
        b["questions"],
        json!({
            "c": {"type": "choice", "instructions": "Tone?", "criteria": {"angry": "upset", "calm": "neutral"}},
            "n": {"type": "noul", "instructions": "Billing?"},
            "s": {"type": "score", "instructions": "Urgent?", "criteria": ["wait", "today"]},
        })
    );
    let p = build_request(
        MODEL,
        &Entry::object(vec![(Key::Fixed("photo"), Entry::image("public", png(1)))]),
        &[Question::locate("b", "the mug in `photo`")],
        &gate(),
        &limits(),
    )
    .unwrap();
    assert_eq!(
        body(&p)["questions"]["b"],
        json!({"type": "box", "instructions": "the mug in `photo`"})
    );
}

#[test]
fn state_is_a_string_an_object_or_an_array_and_instructions_may_be_structured_or_absent() {
    let st = Entry::object(vec![
        (Key::Fixed("message"), text(TEXT)),
        (Key::Fixed("n"), Entry::number("public", 3.5)),
        (Key::Fixed("ok"), Entry::flag("public", true)),
        (Key::Fixed("none"), Entry::Null),
        (Key::Fixed("rows"), Entry::list(vec![text("a"), text("b")])),
    ]);
    let q = Question::noul_with(
        "q",
        Some(Entry::object(vec![
            (Key::Fixed("question"), Entry::fixed("Is it true?")),
            (
                Key::Fixed("compare"),
                Entry::list(vec![Entry::fixed("a"), Entry::fixed("b")]),
            ),
        ])),
        Some(NoulCriteria {
            when_true: Some(Entry::fixed("yes means this")),
            when_false: Some(Entry::object(vec![(
                Key::Fixed("no"),
                Entry::fixed("means that"),
            )])),
        }),
    );
    let b = body(&build(&st, std::slice::from_ref(&q)).unwrap());
    assert_eq!(
        b["state"],
        json!({"message": TEXT, "n": 3.5, "ok": true, "none": null, "rows": ["a", "b"]})
    );
    assert_eq!(
        b["questions"]["q"],
        json!({
            "type": "noul",
            "instructions": {"question": "Is it true?", "compare": ["a", "b"]},
            "criteria": {"true": "yes means this", "false": {"no": "means that"}},
        })
    );
    // an array state, and no instructions at all
    let b = body(
        &build(
            &Entry::list(vec![text("Hi"), text("Help")]),
            &[Question::noul_with("q", None, None)],
        )
        .unwrap(),
    );
    assert_eq!(b["state"], json!(["Hi", "Help"]));
    assert_eq!(b["questions"]["q"], json!({"type": "noul"}));
    // one noul criterion alone
    let q = Question::noul_with(
        "q",
        None,
        Some(NoulCriteria {
            when_true: Some(Entry::fixed("t")),
            when_false: None,
        }),
    );
    let b = body(&build(&text("x"), &[q]).unwrap());
    assert_eq!(b["questions"]["q"]["criteria"], json!({"true": "t"}));
}

#[test]
fn the_body_is_utf8_json_with_no_byte_order_mark() {
    let p = build(&text("caf\u{e9} \u{1F600} \u{FEFF}"), &[noul()]).unwrap();
    assert!(!p.body().as_bytes().starts_with(&[0xEF, 0xBB, 0xBF]));
    assert!(p.body().starts_with('{'));
}

// ---- the label gate ----

#[test]
fn every_labelled_thing_is_gated_wherever_it_sits() {
    let ok = |st: Entry, q: Question| build(&st, &[q]).is_ok();
    let bad = |label: &str| -> Vec<(&'static str, Entry, Question)> {
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
    };
    for label in ["internal", "private", "secret", "", "unknown-label"] {
        for (what, st, q) in bad(label) {
            assert_eq!(
                build(&st, &[q]).err(),
                Some(RequestError::Gated),
                "{what} at {label:?}"
            );
        }
    }
    for (what, st, q) in bad("public") {
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

// ---- images ----

#[test]
fn an_image_is_one_asset_with_its_format_and_base64_and_the_header_turns_on() {
    let st = Entry::object(vec![(Key::Fixed("door"), Entry::image("public", png(1)))]);
    let p = build(&st, &[noul()]).unwrap();
    assert!(p.images());
    let b = body(&p);
    let reference = b["state"]["door"].as_str().unwrap().to_string();
    let a = &b["assets"][&reference];
    assert_eq!(a["type"], "image");
    assert_eq!(a["format"], "png");
    assert_eq!(a["data_base64"], png(1).to_base64());
    assert!(a.get("part").is_none());
    assert_eq!(b["assets"].as_object().unwrap().len(), 1);
    assert!(!build(&text("x"), &[noul()]).unwrap().images());
}

#[test]
fn an_image_inside_a_sentence_and_in_criteria_is_referenced_by_text() {
    let q = Question::choice_with(
        "g",
        Some(Entry::sentence(vec![
            Entry::fixed("Same as this picture: "),
            Entry::image("public", png(2)),
            Entry::fixed("?"),
        ])),
        vec![
            (Key::Fixed("thumb"), Entry::image("public", png(3))),
            (
                Key::Fixed("palm"),
                Entry::object(vec![
                    (Key::Fixed("description"), Entry::fixed("open")),
                    (
                        Key::Fixed("examples"),
                        Entry::list(vec![Entry::image("public", png(4))]),
                    ),
                ]),
            ),
            (Key::Fixed("none"), Entry::fixed("no hand")),
        ],
    );
    let b = body(&build(&text("x"), &[q]).unwrap());
    let assets = b["assets"].as_object().unwrap();
    assert_eq!(assets.len(), 3);
    let instr = b["questions"]["g"]["instructions"].as_str().unwrap();
    assert!(instr.starts_with("Same as this picture: ") && instr.ends_with('?'));
    // every asset is referenced exactly where it belongs, and no reference is a prefix of another
    let rest = b["questions"].to_string() + &b["state"].to_string();
    for k in assets.keys() {
        assert!(rest.contains(k.as_str()), "an asset is not referenced");
        assert_eq!(
            assets.keys().filter(|o| o.starts_with(k.as_str())).count(),
            1
        );
    }
}

#[test]
fn the_same_image_twice_is_one_asset_and_two_uses() {
    let st = Entry::list(vec![
        Entry::image("public", png(5)),
        Entry::image("public", png(5)),
    ]);
    let b = body(&build(&st, &[noul()]).unwrap());
    assert_eq!(b["assets"].as_object().unwrap().len(), 1);
    assert_eq!(b["state"][0], b["state"][1]);
}

#[test]
fn an_image_is_gated_like_text_and_a_reference_lookalike_in_text_is_refused() {
    let r = build(&Entry::image("internal", png(1)), &[noul()]);
    assert_eq!(r.err(), Some(RequestError::Gated));
    let r = build(&text("see klatch-image-0000-end"), &[noul()]);
    assert_eq!(r.err(), Some(RequestError::ReferenceCollision));
    let q = Question::noul_with("q", Some(Entry::fixed("klatch-image-0000-end")), None);
    assert_eq!(
        build(&text("x"), &[q]).err(),
        Some(RequestError::ReferenceCollision)
    );
}

#[test]
fn the_number_of_images_and_of_uses_is_bounded_by_the_config() {
    let lim = Limits::new(40_000, 2, 3).unwrap();
    let go = |st: Entry| build_request(MODEL, &st, &[noul()], &gate(), &lim);
    assert!(
        go(Entry::list(vec![
            Entry::image("public", png(1)),
            Entry::image("public", png(2))
        ]))
        .is_ok()
    );
    assert_eq!(
        go(Entry::list(
            (1..=3).map(|i| Entry::image("public", png(i))).collect()
        ))
        .err(),
        Some(RequestError::TooManyImages)
    );
    assert!(go(Entry::list(vec![Entry::image("public", png(1)); 3])).is_ok());
    assert_eq!(
        go(Entry::list(vec![Entry::image("public", png(1)); 4])).err(),
        Some(RequestError::TooManyImages)
    );
}

#[test]
fn a_box_question_needs_an_image() {
    let q = Question::locate("b", "the mug");
    assert_eq!(
        build(&text("x"), std::slice::from_ref(&q)).err(),
        Some(RequestError::BoxWithoutImage)
    );
    let st = Entry::object(vec![(Key::Fixed("p"), Entry::image("public", png(1)))]);
    assert!(build(&st, &[q]).is_ok());
}

// ---- the documented request rules ----

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

// ---- questions ----

#[test]
fn question_rules_are_checked_before_sending() {
    let st = text("x");
    let r = |qs: &[Question]| build(&st, qs).err();
    assert_eq!(r(&[]), Some(RequestError::NoQuestions));
    assert_eq!(r(&[noul(), noul()]), Some(RequestError::DuplicateQuestion));
    assert!(matches!(
        r(&[Question::noul("", "x")]),
        Some(RequestError::BadQuestion(_))
    ));
    assert!(matches!(
        r(&[Question::noul("  ", "x")]),
        Some(RequestError::BadQuestion(_))
    ));
    let opts = |n: usize| -> Question {
        Question::choice_with(
            "q",
            None,
            (0..n)
                .map(|i| {
                    (
                        Key::Data {
                            label: "public".into(),
                            name: format!("o{i}"),
                        },
                        Entry::Null,
                    )
                })
                .collect(),
        )
    };
    assert!(matches!(r(&[opts(0)]), Some(RequestError::BadQuestion(_))));
    assert_eq!(r(&[opts(1)]), None);
    assert_eq!(r(&[opts(255)]), None);
    assert!(matches!(
        r(&[opts(256)]),
        Some(RequestError::BadQuestion(_))
    ));
    let dup = Question::choice_with(
        "q",
        None,
        vec![
            (Key::Fixed("a"), Entry::Null),
            (Key::Fixed("a"), Entry::Null),
        ],
    );
    assert!(matches!(r(&[dup]), Some(RequestError::BadQuestion(_))));
    let empty = Question::choice_with("q", None, vec![(Key::Fixed(""), Entry::Null)]);
    assert!(matches!(r(&[empty]), Some(RequestError::BadQuestion(_))));
    let levels =
        |n: usize| Question::score_with("q", None, (0..n).map(|_| Entry::fixed("l")).collect());
    assert!(
        matches!(r(&[levels(1)]), Some(RequestError::BadQuestion(_))),
        "a one level scale says nothing"
    );
    assert_eq!(r(&[levels(2)]), None);
    assert_eq!(r(&[levels(10)]), None);
    assert!(matches!(
        r(&[levels(11)]),
        Some(RequestError::BadQuestion(_))
    ));
}

#[test]
fn a_model_that_is_not_pinned_is_never_sent() {
    // the config is the gate for the name; the builder refuses an empty one all the same
    assert!(build_request("", &text("x"), &[noul()], &gate(), &limits()).is_err());
}

#[test]
fn an_image_request_also_carries_the_multipart_form_where_each_asset_names_its_part() {
    let st = Entry::object(vec![
        (Key::Fixed("a"), Entry::image("public", png(1))),
        (Key::Fixed("b"), Entry::image("public", png(2))),
    ]);
    let p = build(&st, &[noul()]).unwrap();
    let (json, files) = p
        .multipart()
        .expect("an image request has a multipart form");
    let v: Value = serde_json::from_str(json).unwrap();
    let assets = v["assets"].as_object().unwrap();
    assert_eq!(assets.len(), 2);
    assert_eq!(files.len(), 2);
    let mut names = BTreeSet::new();
    for (reference, a) in assets {
        assert_eq!(a["type"], "image");
        assert!(a.get("data_base64").is_none(), "no base64 beside a part");
        let part = a["part"].as_str().unwrap();
        assert!(!part.is_empty() && part != "request");
        assert!(names.insert(part.to_string()), "part names are unique");
        assert!(v["state"].to_string().contains(reference.as_str()));
        let f = files
            .iter()
            .find(|f| f.name() == part)
            .expect("file part exists");
        assert_eq!(f.content_type(), "image/png");
        assert_eq!(a["format"], "png");
    }
    assert_eq!(files[0].bytes(), png(1).bytes());
    assert_eq!(files[1].bytes(), png(2).bytes());
}

#[test]
fn a_text_only_request_has_no_multipart_form() {
    assert!(build(&text("x"), &[noul()]).unwrap().multipart().is_none());
}

#[test]
fn the_size_check_uses_what_is_sent_so_multipart_is_smaller_than_base64() {
    // two 1 MiB images are about 2.7 MiB as base64 and 2 MiB as files
    let (json_len, file_len) = (1000usize, 2 * 1024 * 1024usize);
    assert!(multipart_len(json_len, &[file_len / 2, file_len / 2]) < file_len * 4 / 3);
    assert!(multipart_len(json_len, &[file_len / 2, file_len / 2]) > file_len);
}
