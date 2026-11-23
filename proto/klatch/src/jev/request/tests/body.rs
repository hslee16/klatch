use super::*;

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
