use super::*;

#[test]
fn the_first_frame_is_hello_with_the_transports_identity() {
    let g = gateway();
    let f = run(&g, &alice(), &[]);
    assert_eq!(f.len(), 1);
    assert!(
        matches!(&f[0], ServerFrame::Hello { member, attribution: Attribution::Unattributed, .. } if member == "alice")
    );
    let f = run(&g, &Principal::verified("carol"), &[]);
    assert!(matches!(
        &f[0],
        ServerFrame::Hello {
            attribution: Attribution::Verified,
            ..
        }
    ));
}

#[test]
fn every_request_gets_exactly_one_response_that_echoes_its_id_in_order() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[
            req("a", "post", json!({"room":"r","body":"x"})),
            req("b", "who", json!({"room":"r"})),
            req("c", "claims", json!({"room":"r"})),
        ],
    );
    let ids: Vec<&str> = f
        .iter()
        .filter_map(|x| {
            if let ServerFrame::Response { id, .. } = x {
                Some(id.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(ids, vec!["a", "b", "c"]);
}

#[test]
fn post_then_read_round_trips_and_the_sender_is_the_session_principal() {
    let g = gateway();
    run(
        &g,
        &alice(),
        &[req("p", "post", json!({"room":"r","body":"hello team"}))],
    );
    let f = run(
        &g,
        &bob(),
        &[
            req("j", "who", json!({"room":"r"})),
            req("r", "read", json!({"room":"r"})),
        ],
    );
    let (ok, result, _) = response(&f, "r");
    assert!(ok);
    let m = &result.unwrap()["messages"][0];
    assert_eq!(
        (m["sender"].as_str(), m["body"].as_str(), m["seq"].as_u64()),
        (Some("alice"), Some("hello team"), Some(1))
    );
    assert_eq!(m["attribution"], "unattributed");
    assert!(m["id"].as_str().unwrap().len() == 26);
}

#[test]
fn a_request_cannot_choose_who_it_acts_as() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[req(
            "p",
            "post",
            json!({"room":"r","body":"x","member":"bob","sender":"bob"}),
        )],
    );
    assert!(response(&f, "p").0);
    let f = run(&g, &bob(), &[req("r", "read", json!({"room":"r"}))]);
    let (_, result, _) = response(&f, "r");
    assert_eq!(result.unwrap()["messages"][0]["sender"], "alice");
}

#[test]
fn read_advances_the_cursor_and_peek_does_not() {
    let g = gateway();
    run(
        &g,
        &alice(),
        &[req("p", "post", json!({"room":"r","body":"x"}))],
    );
    let f = run(
        &g,
        &bob(),
        &[
            req("1", "read", json!({"room":"r","peek":true})),
            req("2", "read", json!({"room":"r","peek":true})),
            req("3", "read", json!({"room":"r"})),
            req("4", "read", json!({"room":"r"})),
        ],
    );
    let n = |id: &str| {
        response(&f, id).1.unwrap()["messages"]
            .as_array()
            .unwrap()
            .len()
    };
    assert_eq!((n("1"), n("2"), n("3"), n("4")), (1, 1, 1, 0));
}

#[test]
fn history_returns_the_newest_messages_and_how_many_more_follow_and_moves_no_cursor() {
    let g = gateway();
    let posts: Vec<Value> = (1..=5)
        .map(|i| req("p", "post", json!({"room":"r","body":format!("m{i}")})))
        .collect();
    run(&g, &alice(), &posts);
    let f = run(
        &g,
        &bob(),
        &[
            req("h", "history", json!({"room":"r","last":2})),
            req("s", "history", json!({"room":"r","since":1,"last":2})),
            req("f", "history", json!({"room":"r","since":0})),
            req("bad", "history", json!({"room":"r","last":0})),
            req("u", "read", json!({"room":"r","peek":true})),
        ],
    );
    let h = response(&f, "h").1.unwrap();
    assert_eq!(h["messages"][0]["body"], "m4");
    assert_eq!(h["messages"][1]["body"], "m5");
    assert_eq!(h["more"], 0);
    assert_eq!(h["messages"][0]["attribution"], "unattributed");
    assert_eq!(response(&f, "s").1.unwrap()["messages"][0]["body"], "m4");
    assert_eq!(
        response(&f, "f").1.unwrap()["messages"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert!(!response(&f, "bad").0, "last 0 is refused");
    assert_eq!(
        response(&f, "u").1.unwrap()["messages"]
            .as_array()
            .unwrap()
            .len(),
        5,
        "history left every message unread"
    );
}

#[test]
fn invite_posts_a_pointer_and_is_refused_for_a_member_who_is_not_in_the_room() {
    let g = gateway();
    run(
        &g,
        &alice(),
        &[req("j", "post", json!({"room":"r","body":"x"}))],
    );
    run(
        &g,
        &bob(),
        &[req("j", "post", json!({"room":"r","body":"y"}))],
    );
    let f = run(
        &g,
        &alice(),
        &[
            req(
                "i",
                "invite",
                json!({"member":"bob","room":"new","via":"r"}),
            ),
            req(
                "n",
                "invite",
                json!({"member":"zed","room":"new","via":"r"}),
            ),
            req(
                "self",
                "invite",
                json!({"member":"alice","room":"new","via":"r"}),
            ),
        ],
    );
    assert!(
        response(&f, "i").0,
        "an invite of a member in the room works"
    );
    assert!(!response(&f, "n").0, "nobody to reach in that room");
    assert!(!response(&f, "self").0, "not yourself");
    let got = run(&g, &bob(), &[req("r", "read", json!({"room":"r"}))]);
    let body = response(&got, "r").1.unwrap()["messages"].to_string();
    assert!(body.contains("klatch join new"), "{body}");
}

#[test]
fn who_lists_members_with_last_seen() {
    let g = gateway();
    run(
        &g,
        &alice(),
        &[req("j", "post", json!({"room":"r","body":"x"}))],
    );
    let f = run(&g, &bob(), &[req("w", "who", json!({"room":"r"}))]);
    let members = response(&f, "w").1.unwrap()["members"].clone();
    assert_eq!(members[0]["name"], "alice");
    assert_eq!(members[0]["last_seen"], 1000);
}

#[test]
fn claims_conflict_release_and_list_over_the_wire() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[req(
            "c",
            "claim",
            json!({"room":"r","path":"src","reason":"refactor"}),
        )],
    );
    assert!(response(&f, "c").0);
    let f = run(
        &g,
        &bob(),
        &[
            req("c", "claim", json!({"room":"r","path":"src/a.rs"})),
            req("l", "claims", json!({"room":"r"})),
        ],
    );
    let (ok, _, err) = response(&f, "c");
    assert!(!ok);
    let (code, msg) = err.unwrap();
    assert_eq!(code, "conflict");
    assert!(msg.contains("src is claimed by alice"), "{msg}");
    let claims = response(&f, "l").1.unwrap()["claims"].clone();
    assert_eq!(
        (
            claims[0]["path"].as_str(),
            claims[0]["owner"].as_str(),
            claims[0]["reason"].as_str()
        ),
        (Some("src"), Some("alice"), Some("refactor"))
    );
    let f = run(
        &g,
        &alice(),
        &[req("r", "release", json!({"room":"r","path":"src"}))],
    );
    assert!(response(&f, "r").0);
    let f = run(
        &g,
        &bob(),
        &[req("c", "claim", json!({"room":"r","path":"src/a.rs"}))],
    );
    assert!(response(&f, "c").0);
}

#[test]
fn a_claim_defaults_its_ttl_and_honours_an_explicit_one() {
    let g = gateway();
    run(
        &g,
        &alice(),
        &[
            req("a", "claim", json!({"room":"r","path":"one"})),
            req("b", "claim", json!({"room":"r","path":"two","ttl":30})),
        ],
    );
    let f = run(&g, &bob(), &[req("l", "claims", json!({"room":"r"}))]);
    let c = response(&f, "l").1.unwrap()["claims"].clone();
    assert_eq!(c[0]["expires"], 1000 + 600);
    assert_eq!(c[1]["expires"], 1000 + 30);
}

#[test]
fn releasing_a_claim_you_do_not_hold_is_not_found() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[req("r", "release", json!({"room":"r","path":"src"}))],
    );
    assert_eq!(response(&f, "r").2.unwrap().0, "not_found");
}

#[test]
fn invalid_input_and_unknown_methods_get_distinct_error_codes() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[
            req("1", "post", json!({"body":"no room"})),
            req("2", "post", json!({"room":"bad room","body":"x"})),
            req("3", "teleport", json!({})),
            json!({"v":1,"type":"request","id":"4","method":"who"}),
        ],
    );
    assert_eq!(response(&f, "1").2.unwrap().0, "invalid_params");
    assert_eq!(response(&f, "2").2.unwrap().0, "invalid");
    assert_eq!(response(&f, "3").2.unwrap().0, "unknown_method");
    assert_eq!(response(&f, "4").2.unwrap().0, "invalid_params");
}

struct DenyRoom(&'static str);
impl Guard for DenyRoom {
    fn check(&self, _who: &Principal, op: &Operation) -> Result<()> {
        if let Operation::Post { room, .. } = op
            && room == self.0
        {
            return Err(Error::Refused {
                rule: "test.deny".into(),
                reason: "that room is closed".into(),
            });
        }
        Ok(())
    }
}

#[test]
fn a_policy_refusal_sends_a_refusal_frame_then_a_failed_response_and_never_echoes_the_body() {
    let g = Gateway::new(
        SqliteStore::open_in_memory().unwrap(),
        FixedClock(1),
        DenyRoom("locked"),
    );
    let f = run(
        &g,
        &alice(),
        &[req(
            "p",
            "post",
            json!({"room":"locked","body":"SECRET-BODY-TEXT"}),
        )],
    );
    assert!(
        matches!(&f[1], ServerFrame::Refusal { request: Some(r), rule, .. } if r == "p" && rule == "test.deny")
    );
    let (ok, _, err) = response(&f, "p");
    assert!(!ok);
    assert_eq!(err.unwrap().0, "refused");
    assert!(!format!("{f:?}").contains("SECRET-BODY-TEXT"));
}

#[test]
fn garbage_and_wrong_version_lines_are_refused_and_the_session_carries_on() {
    let g = gateway();
    let input = "this is not json\n{\"v\":2,\"type\":\"request\",\"id\":\"x\",\"method\":\"who\"}\n"
        .to_string() + &format!("{}\n", req("ok", "who", json!({"room":"r"})));
    let mut out = Vec::new();
    serve(&g, &alice(), Cursor::new(input), &mut out).unwrap();
    let f: Vec<ServerFrame> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| decode_server_line(l).unwrap())
        .collect();
    let rules: Vec<&str> = f
        .iter()
        .filter_map(|x| {
            if let ServerFrame::Refusal { rule, .. } = x {
                Some(rule.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(rules, vec!["protocol.decode", "protocol.version"]);
    assert!(response(&f, "ok").0);
}

#[test]
fn an_oversize_line_is_refused_and_the_session_carries_on() {
    let g = gateway();
    let huge = "x".repeat(crate::protocol::MAX_LINE_BYTES * 2);
    let input = format!("{huge}\n{}\n", req("ok", "who", json!({"room":"r"})));
    let mut out = Vec::new();
    serve(&g, &alice(), Cursor::new(input), &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("protocol.decode"));
    assert!(text.contains("\"id\":\"ok\""));
}

#[test]
fn acks_and_unknown_frames_are_accepted_silently() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[
            json!({"v":1,"type":"ack","room":"r","seq":3}),
            json!({"v":1,"type":"something_new","x":1}),
        ],
    );
    assert_eq!(f.len(), 1, "only hello: {f:?}");
}

pub(super) struct DownStore;
impl MessageStore for DownStore {
    fn join(&self, _: &str, _: &str, _: i64) -> Result<()> {
        Err(Error::Backend("down".into()))
    }

    fn post_as(&self, _: &NewMessage, _: i64) -> Result<Posted> {
        Err(Error::Backend("down".into()))
    }

    fn read(&self, _: Seat, _: bool, _: i64) -> Result<Vec<Message>> {
        Err(Error::Busy("locked".into()))
    }

    fn tail(&self, _: &str, _: i64, _: usize) -> Result<Vec<Message>> {
        Err(Error::Backend("down".into()))
    }

    fn head(&self, _: &str) -> Result<i64> {
        Err(Error::Backend("down".into()))
    }

    fn advance_cursor(&self, _: Seat, _: Seq, _: i64) -> Result<()> {
        Err(Error::Backend("down".into()))
    }
}
impl MemberStore for DownStore {
    fn members(&self, _: &str) -> Result<Vec<MemberInfo>> {
        Err(Error::Backend("down".into()))
    }

    fn rooms_of(&self, _: &str) -> Result<Vec<String>> {
        Err(Error::Backend("down".into()))
    }

    fn is_blocked(&self, _: &str, _: &str) -> Result<bool> {
        Ok(false)
    }
}
impl ClaimStore for DownStore {
    fn claim(&self, _: &ClaimRequest, _: i64) -> Result<()> {
        Err(Error::Backend("down".into()))
    }

    fn release(&self, _: &str, _: &str, _: &str) -> Result<()> {
        Err(Error::Backend("down".into()))
    }

    fn claims(&self, _: &str, _: i64) -> Result<Vec<Claim>> {
        Err(Error::Backend("down".into()))
    }
}
impl AskStore for DownStore {}
impl ReviewStore for DownStore {}
impl CredentialStore for DownStore {}
// the checks that run before the message store is reached see an empty, healthy policy, so each failure is the one
// the message store gives (the silent defaults are gone, #485)
impl PolicyStore for DownStore {
    fn policy_get(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
    fn policy_list(&self, _: &str) -> Result<Vec<(String, String)>> {
        Ok(Vec::new())
    }
}
impl RoomStore for DownStore {
    fn room_state(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
    fn set_label_keys(&self, _keys: crate::keys::LabelKeys) {}
}

#[test]
fn store_failures_become_failed_responses_and_the_session_survives() {
    let g = Gateway::new(DownStore, FixedClock(1), AllowAll);
    let f = run(
        &g,
        &alice(),
        &[
            req("1", "post", json!({"room":"r","body":"x"})),
            req("2", "read", json!({"room":"r"})),
            req("3", "who", json!({"room":"r"})),
        ],
    );
    assert_eq!(response(&f, "1").2.unwrap().0, "backend");
    assert_eq!(response(&f, "2").2.unwrap().0, "busy");
    assert_eq!(response(&f, "3").2.unwrap().0, "backend");
}

#[test]
fn the_session_ends_cleanly_at_end_of_input() {
    let g = gateway();
    let mut out = Vec::new();
    assert!(serve(&g, &alice(), Cursor::new(""), &mut out).is_ok());
}

#[test]
fn every_output_line_is_one_valid_frame() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[
            req("p", "post", json!({"room":"r","body":"line one\nline two"})),
            req("r", "read", json!({"room":"r"})),
        ],
    );
    assert!(f.len() >= 3); // decoding every line already proved validity
}

struct MovingClock(std::rc::Rc<std::cell::Cell<i64>>);
impl crate::gateway::Clock for MovingClock {
    fn now(&self) -> i64 {
        self.0.get()
    }
}

#[test]
fn a_session_for_a_verified_member_whose_credential_has_ended_is_refused_on_every_request() {
    // the process proved itself while the credential was live and kept running (#161): the session checks again
    let now = std::rc::Rc::new(std::cell::Cell::new(1_000));
    let g = Gateway::new(
        SqliteStore::open_in_memory().unwrap(),
        MovingClock(now.clone()),
        AllowAll,
    );
    let spec = crate::gateway::CredentialSpec {
        member: "w-1",
        clearance: "internal",
        ttl: 10,
        max_life: None,
        parent: None,
    };
    g.admin_issue_credential(&alice(), &spec).unwrap();
    let who = Principal::verified("w-1");
    let live = run(
        &g,
        &who,
        &[req("a", "post", json!({"room":"r","body":"x"}))],
    );
    assert!(response(&live, "a").0, "a live credential was refused");
    now.set(1_010);
    let f = run(
        &g,
        &who,
        &[
            req("a", "post", json!({"room":"r","body":"y"})),
            req("b", "who", json!({"room":"r"})),
            req("c", "claims", json!({"room":"r"})),
        ],
    );
    for id in ["a", "b", "c"] {
        let (ok, _, err) = response(&f, id);
        assert!(!ok, "{id} was answered after the credential expired");
        assert_eq!(err.unwrap().0, "refused");
    }
}
