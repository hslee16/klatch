use super::*;

// ---- config ----

#[test]
fn the_model_must_be_a_pinned_jev_version_never_latest() {
    for bad in [
        "",
        "jev-latest",
        "jev-preview",
        "latest",
        "JEV-LATEST",
        "1.14.0",
        "jev-1.14",
        "jev-1.x.0",
        "jev-1.14.0.1",
        "jev-1..0",
        "jev--1.14.0",
        "jev-jev-1.14.0",
        " jev-1.14.0",
        "jev-1.14.0 ",
        "jev-1.14.0-beta",
    ] {
        assert!(JevConfig::new(bad, 0.8).is_err(), "{bad:?}");
    }
    assert!(JevConfig::new("jev-1.14.0", 0.8).is_ok());
    assert!(JevConfig::new("jev-1.13.0", 0.8).is_ok());
}

#[test]
fn the_threshold_must_be_in_zero_exclusive_to_one() {
    for bad in [0.0, -0.1, 1.1, f64::NAN, f64::INFINITY] {
        assert!(JevConfig::new(MODEL, bad).is_err(), "{bad}");
    }
    assert!(JevConfig::new(MODEL, 1.0).is_ok());
}

#[test]
fn the_limits_have_a_safe_default_and_can_be_replaced() {
    assert_eq!(cfg().limits(), &Limits::default());
    let l = Limits::new(1000, 1, 2).unwrap();
    assert_eq!(cfg().with_limits(l).limits(), &l);
}

#[test]
fn listing_an_alias_never_makes_the_config_accept_it() {
    // the listing names jev-latest and jev-preview; the config still refuses both
    let list = parse_model_list(fixtures::LIVE_MODELS).unwrap();
    for m in list.models() {
        assert!(JevConfig::new(&m.name, 0.8).is_err(), "{}", m.name);
    }
}

// ---- asking ----

#[test]
fn the_request_goes_to_the_system_one_path_with_the_key_and_the_documented_body() {
    let f = FakeTransport::saying(Ok(fixtures::LIVE_CHOICE.to_string()));
    let a = ask_judge(&f).unwrap();
    assert_eq!(a.choice("verdict").unwrap().choice, "allow");
    let seen = f.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        (seen[0].method, seen[0].path),
        (Method::Post, "/v1/systemone")
    );
    assert_eq!(seen[0].key, SECRET);
    assert!(!seen[0].images);
    let v: Value = serde_json::from_str(seen[0].body.as_ref().unwrap()).unwrap();
    assert_eq!(v["model"], MODEL);
    assert_eq!(v["state"]["message"], TEXT);
    assert!(v["questions"].is_object());
    assert_eq!(
        JevRequest::system_one(
            &build_request(
                MODEL,
                &state(),
                &[judge_question()],
                &gate(),
                &Limits::default()
            )
            .unwrap()
        )
        .headers(),
        vec![("Content-Type", "application/json")]
    );
}

#[test]
fn a_request_with_an_image_or_a_box_question_turns_the_images_header_on() {
    let f = FakeTransport::saying(Ok(fixtures::DOCS_BOX.to_string()));
    let st = Entry::object(vec![(Key::Fixed("photo"), Entry::image("public", png()))]);
    let a = client(&f)
        .ask(&st, &[Question::locate("mug", "the mug in `photo`")])
        .unwrap();
    assert_eq!(a.boxed("mug").unwrap().confidence, Some(0.92));
    assert!(f.seen()[0].images);
    let req = JevRequest {
        method: Method::Post,
        path: "/v1/systemone",
        images: true,
        body: Some(String::new()),
        files: vec![],
    };
    assert!(
        req.headers()
            .contains(&("TypeSafe-Danger-Zone-Images", "true"))
    );
    assert!(JevRequest::models().headers().is_empty());
}

#[test]
fn all_four_kinds_round_trip_through_the_client() {
    let reply = json!({
        "model": MODEL,
        "answers": {
            "c": {"type": "choice", "choice": "a", "confidence": 0.9, "probabilities": {"a": 0.9, "b": 0.1}},
            "n": {"type": "noul", "noul": 0.2},
            "s": {"type": "score", "score": 0.5, "confidence": 0.0, "legend": {"0": "x", "1": "y"}, "probabilities": {"0": 0.5, "1": 0.5}},
            "b": {"type": "box", "box": {"top": 0.1, "bottom": 0.9, "left": 0.1, "right": 0.9}, "confidence": null},
        },
        "usage": {"input_tokens": 1, "output_tokens": 1},
    });
    let f = FakeTransport::saying(Ok(reply.to_string()));
    let st = Entry::object(vec![(Key::Fixed("p"), Entry::image("public", png()))]);
    let qs = [
        Question::choice("c", "?", &[("a", "x"), ("b", "y")]),
        Question::noul("n", "?"),
        Question::score("s", "?", &["x", "y"]),
        Question::locate("b", "?"),
    ];
    let a = client(&f).ask(&st, &qs).unwrap();
    assert_eq!(a.noul("n"), Some(0.2));
    assert_eq!(a.score("s").unwrap().probabilities, vec![0.5, 0.5]);
    assert_eq!(a.boxed("b").unwrap().confidence, None);
}

#[test]
fn a_refused_request_sends_nothing_and_never_asks_for_the_key() {
    let f = FakeTransport::saying(Ok(fixtures::LIVE_CHOICE.to_string()));
    let asked = Rc::new(Cell::new(0));
    let a = asked.clone();
    let c = JevClient::new(&f, cfg())
        .with_gate(gate())
        .with_key_source(Box::new(move || {
            a.set(a.get() + 1);
            Some(SECRET.into())
        }));
    // above the ceiling, a repeated id, and far more text than the bound (the live check saw 500,000 refused)
    let secret = Entry::object(vec![(Key::Fixed("m"), Entry::text("private", "LEAK"))]);
    assert_eq!(
        c.ask(&secret, &[judge_question()]).unwrap_err(),
        JevError::Refused(RequestError::Gated)
    );
    let dup = [judge_question(), judge_question()];
    assert_eq!(
        c.ask(&state(), &dup).unwrap_err(),
        JevError::Refused(RequestError::DuplicateQuestion)
    );
    let big = Entry::text("public", &"x".repeat(100_000));
    assert_eq!(
        c.ask(&big, &[judge_question()]).unwrap_err(),
        JevError::Refused(RequestError::TooManyChars)
    );
    let huge = Entry::text("public", &"x".repeat(500_000));
    assert_eq!(
        c.ask(&huge, &[judge_question()]).unwrap_err(),
        JevError::Refused(RequestError::TooManyChars)
    );
    assert_eq!(f.calls(), 0);
    assert_eq!(asked.get(), 0);
}

#[test]
fn a_default_client_has_a_closed_gate_so_labelled_data_never_leaves() {
    let f = FakeTransport::saying(Ok(fixtures::LIVE_CHOICE.to_string()));
    let c = JevClient::new(&f, cfg()).with_key_source(Box::new(|| Some(SECRET.into())));
    assert_eq!(
        c.ask(&state(), &[judge_question()]).unwrap_err(),
        JevError::Refused(RequestError::Gated)
    );
    assert_eq!(f.calls(), 0);
}

// ---- key handling ----

#[test]
fn the_key_is_read_from_typesafe_api_key_only() {
    assert_eq!(KEY_ENV, "TYPESAFE_API_KEY");
    let got = ApiKey::from_lookup(|n| (n == "TYPESAFE_API_KEY").then(|| SECRET.to_string()));
    assert_eq!(got.unwrap().expose(), SECRET);
    assert!(ApiKey::from_lookup(|_| None).is_none());
    assert!(ApiKey::from_lookup(|_| Some(String::new())).is_none());
    assert!(ApiKey::from_lookup(|_| Some("  ".into())).is_none());
}

#[test]
fn the_key_is_read_at_each_call_not_cached() {
    let f = FakeTransport::saying(Ok(judge_reply("allow", 0.95)));
    let n = Cell::new(0);
    let c = JevClient::new(&f, cfg())
        .with_gate(gate())
        .with_key_source(Box::new(move || {
            n.set(n.get() + 1);
            Some(format!("key-{}", n.get()))
        }));
    c.ask(&state(), &[judge_question()]).unwrap();
    c.ask(&state(), &[judge_question()]).unwrap();
    let keys: Vec<String> = f.seen().into_iter().map(|s| s.key).collect();
    assert_eq!(keys, vec!["key-1", "key-2"]);
}

#[test]
fn debug_and_errors_never_contain_the_key() {
    let key = ApiKey::new(SECRET.to_string());
    assert!(!format!("{key:?}").contains(SECRET));
    assert!(!format!("{key}").contains(SECRET));
    let f = FakeTransport::saying(Err(JevTransportError::Network));
    let err = ask_judge(&f).unwrap_err();
    assert!(!format!("{err} {err:?}").contains(SECRET));
    let f = FakeTransport::saying(Ok(format!("oops {SECRET}")));
    let err = ask_judge(&f).unwrap_err();
    assert!(!format!("{err} {err:?}").contains(SECRET));
    assert!(!format!("{:?}", client(&f).config()).contains(SECRET));
    assert!(!format!("{:?}", JevJudge::new(client(&f)).config()).contains(SECRET));
}

#[test]
fn a_missing_key_means_no_call() {
    let f = FakeTransport::saying(Ok(judge_reply("allow", 0.95)));
    let c = JevClient::new(&f, cfg())
        .with_gate(gate())
        .with_key_source(Box::new(|| None));
    assert_eq!(
        c.ask(&state(), &[judge_question()]).unwrap_err(),
        JevError::NoKey
    );
    assert_eq!(c.list_models().unwrap_err(), JevError::NoKey);
    assert_eq!(f.calls(), 0);
}

#[test]
fn the_key_is_never_in_the_request_body() {
    let f = FakeTransport::saying(Ok(judge_reply("allow", 0.95)));
    hosted(&f).judge(&item("public", TEXT));
    assert!(!f.last_body().unwrap().contains(SECRET));
}
