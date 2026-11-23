use super::*;

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
