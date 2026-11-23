use super::*;

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
