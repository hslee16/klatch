//! The `dm` wire method (#345): a direct message to one other member, over the same post path as any room.

use super::*;

fn admin() -> Principal {
    Principal::unattributed("admin")
}

fn carol() -> Principal {
    Principal::unattributed("carol")
}

/// Send one `dm` request as `who`; the response triple of `response`.
fn dm(
    g: &Gateway<SqliteStore>,
    who: &Principal,
    params: Value,
) -> (bool, Option<Value>, Option<(String, String)>) {
    let f = run(g, who, &[req("d", "dm", params)]);
    response(&f, "d")
}

fn code_of(r: &(bool, Option<Value>, Option<(String, String)>)) -> String {
    r.2.clone().expect("a failure").0
}

fn rule_in(frames: &[ServerFrame]) -> Option<String> {
    frames.iter().find_map(|f| match f {
        ServerFrame::Refusal { rule, .. } => Some(rule.clone()),
        _ => None,
    })
}

#[test]
fn a_direct_message_over_the_wire_reaches_the_other_member_in_a_room_both_can_read() {
    let g = gateway();
    let (ok, result, _) = dm(&g, &alice(), json!({"to":"bob","body":"hello bob"}));
    assert!(ok);
    let r = result.unwrap();
    assert_eq!(r["room"], "dm.alice.bob");
    assert_eq!(r["seq"], 1);
    assert_eq!(r["id"].as_str().unwrap().len(), 26);
    let f = run(
        &g,
        &bob(),
        &[req("r", "read", json!({"room":"dm.alice.bob"}))],
    );
    let m = &response(&f, "r").1.unwrap()["messages"][0];
    assert_eq!(
        (m["sender"].as_str(), m["body"].as_str()),
        (Some("alice"), Some("hello bob"))
    );
}

#[test]
fn the_result_names_exactly_the_room_the_seq_and_the_id() {
    let g = gateway();
    let (_, result, _) = dm(&g, &alice(), json!({"to":"bob","body":"x"}));
    let mut keys: Vec<String> = result
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    keys.sort();
    assert_eq!(keys, vec!["id", "room", "seq"]);
}

#[test]
fn a_reply_lands_in_the_same_room_with_the_next_seq() {
    let g = gateway();
    dm(&g, &alice(), json!({"to":"bob","body":"one"}));
    let (_, result, _) = dm(&g, &bob(), json!({"to":"alice","body":"two"}));
    let r = result.unwrap();
    assert_eq!(
        (r["room"].as_str(), r["seq"].as_u64()),
        (Some("dm.alice.bob"), Some(2))
    );
}

#[test]
fn the_recipients_digest_shows_it_like_any_room() {
    let g = gateway();
    dm(&g, &alice(), json!({"to":"bob","body":"look at this"}));
    let d = g.digest(&bob()).unwrap();
    assert_eq!(d.rooms[0].room, "dm.alice.bob");
}

#[test]
fn a_sender_or_member_param_cannot_change_who_sends() {
    let g = gateway();
    let (_, result, _) = dm(
        &g,
        &alice(),
        json!({"to":"bob","body":"x","member":"carol","sender":"carol","from":"carol"}),
    );
    assert_eq!(result.unwrap()["room"], "dm.alice.bob");
}

#[test]
fn a_third_member_is_refused_the_room_over_the_wire() {
    let g = gateway();
    dm(&g, &alice(), json!({"to":"bob","body":"private talk"}));
    let f = run(
        &g,
        &carol(),
        &[req("r", "read", json!({"room":"dm.alice.bob"}))],
    );
    assert_eq!(rule_in(&f).as_deref(), Some("dm-members"));
    let (ok, _, err) = response(&f, "r");
    assert!(!ok);
    assert_eq!(err.unwrap().0, "refused");
    let f = run(
        &g,
        &carol(),
        &[req("p", "post", json!({"room":"dm.alice.bob","body":"hi"}))],
    );
    assert_eq!(rule_in(&f).as_deref(), Some("dm-members"));
}

#[test]
fn a_dm_room_name_nobody_made_stays_reserved_over_the_wire() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[req(
            "p",
            "post",
            json!({"room":"dm.alice.bob","body":"squat"}),
        )],
    );
    assert_eq!(rule_in(&f).as_deref(), Some("dm-reserved"));
}

#[test]
fn a_message_to_oneself_a_bad_name_or_an_empty_body_is_invalid() {
    let g = gateway();
    for params in [
        json!({"to":"alice","body":"x"}),
        json!({"to":"bad name","body":"x"}),
        json!({"to":"","body":"x"}),
        json!({"to":"bob","body":"   "}),
        json!({"to":"bob","body":""}),
    ] {
        let r = dm(&g, &alice(), params.clone());
        assert!(!r.0, "{params}");
        assert_eq!(code_of(&r), "invalid", "{params}");
    }
}

#[test]
fn dm_needs_a_recipient_and_a_body_of_the_right_shape() {
    let g = gateway();
    for params in [
        json!({}),
        json!({"to":"bob"}),
        json!({"body":"x"}),
        json!({"to":5,"body":"x"}),
        json!({"to":"bob","body":5}),
        json!({"to":"bob","body":"x","label":5}),
    ] {
        let r = dm(&g, &alice(), params.clone());
        assert!(!r.0, "{params}");
        assert_eq!(code_of(&r), "invalid_params", "{params}");
    }
}

#[test]
fn the_label_is_capped_at_the_lower_clearance_and_may_go_lower() {
    let g = gateway();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.admin_set_clearance(&admin(), "bob", "internal").unwrap();
    let f = run(
        &g,
        &alice(),
        &[req(
            "d",
            "dm",
            json!({"to":"bob","body":"a private fact","label":"private"}),
        )],
    );
    assert_eq!(rule_in(&f).as_deref(), Some("label"));
    assert_eq!(response(&f, "d").2.unwrap().0, "refused");
    let (ok, _, _) = dm(
        &g,
        &alice(),
        json!({"to":"bob","body":"less so","label":"public"}),
    );
    assert!(ok);
    let f = run(
        &g,
        &bob(),
        &[req("h", "history", json!({"room":"dm.alice.bob"}))],
    );
    let m = &response(&f, "h").1.unwrap()["messages"];
    assert_eq!(m.as_array().unwrap().len(), 1);
    assert_eq!(m[0]["label"], "public");
}

#[test]
fn without_a_label_the_room_ceiling_applies() {
    let g = gateway();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.admin_set_clearance(&admin(), "bob", "internal").unwrap();
    dm(&g, &alice(), json!({"to":"bob","body":"x"}));
    let f = run(
        &g,
        &bob(),
        &[req("h", "history", json!({"room":"dm.alice.bob"}))],
    );
    assert_eq!(
        response(&f, "h").1.unwrap()["messages"][0]["label"],
        "internal"
    );
}

#[test]
fn a_credential_in_a_direct_message_is_refused_and_never_echoed() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[req(
            "d",
            "dm",
            json!({"to":"bob","body":"key AKIAIOSFODNN7EXAMPLE"}),
        )],
    );
    assert_eq!(rule_in(&f).as_deref(), Some("credentials"));
    assert!(!response(&f, "d").0);
    assert!(!format!("{f:?}").contains("AKIAIOSFODNN7EXAMPLE"));
}

#[test]
fn a_scrubbed_term_in_a_direct_message_is_refused_and_never_echoed() {
    let g = gateway();
    g.admin_add_scrub_term(&admin(), "project", "bluebird")
        .unwrap();
    let f = run(
        &g,
        &alice(),
        &[req("d", "dm", json!({"to":"bob","body":"about BLUEBIRD"}))],
    );
    assert_eq!(rule_in(&f).as_deref(), Some("scrub:project"));
    assert!(!format!("{f:?}").to_lowercase().contains("bluebird"));
}

#[test]
fn a_paused_gateway_refuses_a_direct_message_over_the_wire() {
    let g = gateway();
    g.admin_pause(&admin(), true).unwrap();
    let f = run(
        &g,
        &alice(),
        &[req("d", "dm", json!({"to":"bob","body":"x"}))],
    );
    assert_eq!(rule_in(&f).as_deref(), Some("admin-pause"));
}

#[test]
fn a_librarian_may_not_send_a_direct_message_over_the_wire() {
    let g = gateway();
    g.admin_set_librarian(&admin(), "lib", true).unwrap();
    let g = g.for_members();
    let lib = Principal::unattributed("lib");
    let f = run(&g, &lib, &[req("d", "dm", json!({"to":"bob","body":"x"}))]);
    assert_eq!(rule_in(&f).as_deref(), Some("read-only"));
}

#[test]
fn a_direct_message_is_audited_the_same_way_as_a_post_by_reaching_the_guard() {
    let g = Gateway::new(
        SqliteStore::open_in_memory().unwrap(),
        FixedClock(1),
        DenyDmRoom,
    );
    let f = run(
        &g,
        &alice(),
        &[req("d", "dm", json!({"to":"bob","body":"x"}))],
    );
    assert_eq!(rule_in(&f).as_deref(), Some("test.deny-dm"));
    assert_eq!(response(&f, "d").2.unwrap().0, "refused");
}

#[test]
fn the_scrub_rules_and_the_label_order_stay_off_the_wire() {
    let g = gateway();
    for method in [
        "scrub",
        "scrub_add",
        "scrub_remove",
        "scrub_terms",
        "scrub-add",
        "label_chain",
        "chain",
        "cap",
        "set_label_chain",
    ] {
        let f = run(
            &g,
            &alice(),
            &[req("s", method, json!({"name":"x","term":"y"}))],
        );
        assert_eq!(response(&f, "s").2.unwrap().0, "unknown_method", "{method}");
    }
}

struct DenyDmRoom;
impl Guard for DenyDmRoom {
    fn check(&self, _who: &Principal, op: &Operation) -> Result<()> {
        match op {
            Operation::Post { room, .. } if room == "dm.alice.bob" => Err(Error::Refused {
                rule: "test.deny-dm".into(),
                reason: "that room is closed".into(),
            }),
            _ => Ok(()),
        }
    }
}
