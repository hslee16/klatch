//! The role-checked wire methods (#506, #509): `role_act` for the admin agent, `review_next` and `review_verdict`
//! for the reviewer. The method table holds no floor act, and the gateway refuses a member without the role.

use super::serve;
use crate::domain::Principal;
use crate::gateway::{AllowAll, FixedClock, ForbidFloors, Gateway};
use crate::protocol::{ServerFrame, decode_server_line};
use crate::store::areas::*;
use crate::store::{NewReview, SqliteStore};
use serde_json::{Value, json};
use std::io::Cursor;

pub(super) fn who(name: &str) -> Principal {
    Principal::unattributed(name)
}

/// A members' gateway: roles are granted first through the terminal gateway (no `for_members`), as the human does.
pub(super) fn gateway() -> Gateway<SqliteStore> {
    let g = Gateway::new(
        SqliteStore::open_in_memory().unwrap(),
        FixedClock(1_000),
        ForbidFloors::new(AllowAll),
    );
    let human = who("admin");
    g.join(&who("alice"), "r").unwrap();
    g.post(&who("alice"), "r", "candidate text").unwrap();
    g.store_for_tests()
        .enqueue_review(&NewReview::new("r", 1, "public", "crosses"), 1_001)
        .unwrap();
    g.admin_set_admin_agent(&human, "ada", true).unwrap();
    g.admin_set_reviewer(&human, "rev", true).unwrap();
    g.for_members()
}

pub(super) fn call(
    g: &Gateway<SqliteStore>,
    member: &str,
    method: &str,
    params: Value,
) -> Vec<ServerFrame> {
    let line = format!(
        "{}\n",
        json!({"v":1,"type":"request","id":"1","method":method,"params":params})
    );
    let mut out = Vec::new();
    serve(g, &who(member), Cursor::new(line), &mut out).unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| decode_server_line(l).unwrap())
        .collect()
}

/// The result of the one response, or its error code.
pub(super) fn answer(frames: &[ServerFrame]) -> Result<Value, String> {
    for f in frames {
        if let ServerFrame::Response {
            ok, result, error, ..
        } = f
        {
            return match ok {
                true => Ok(result.clone().unwrap()),
                false => Err(error.clone().unwrap().code),
            };
        }
    }
    panic!("no response: {frames:?}");
}

pub(super) fn rule_of(frames: &[ServerFrame]) -> Option<String> {
    frames.iter().find_map(|f| match f {
        ServerFrame::Refusal { rule, .. } => Some(rule.clone()),
        _ => None,
    })
}

#[test]
fn the_admin_agent_calls_its_acts_over_the_wire() {
    let g = gateway();
    let f = call(&g, "ada", "role_act", json!({"act":"close","room":"r"}));
    assert!(answer(&f).is_ok(), "{f:?}");
    let f = call(&g, "alice", "post", json!({"room":"r","body":"x"}));
    assert_eq!(rule_of(&f).as_deref(), Some("admin-close"));
    let f = call(&g, "ada", "role_act", json!({"act":"reopen","room":"r"}));
    assert!(answer(&f).is_ok());
    let f = call(
        &g,
        "ada",
        "role_act",
        json!({"act":"clear","member":"alice","label":"internal"}),
    );
    assert!(answer(&f).is_ok(), "{f:?}");
}

#[test]
fn the_admin_agent_reads_the_records_over_the_wire() {
    let g = gateway();
    let f = call(&g, "ada", "role_act", json!({"act":"audit","limit":5}));
    let v = answer(&f).unwrap();
    assert!(v["records"].is_array(), "{v}");
    let f = call(&g, "ada", "role_act", json!({"act":"refusals"}));
    assert!(answer(&f).unwrap()["records"].is_array());
}

#[test]
fn an_ordinary_member_is_refused_a_role_act() {
    let g = gateway();
    let f = call(&g, "alice", "role_act", json!({"act":"close","room":"r"}));
    assert_eq!(answer(&f), Err("refused".into()));
    assert_eq!(rule_of(&f).as_deref(), Some("admin-scope"));
    let f = call(&g, "alice", "post", json!({"room":"r","body":"x"}));
    assert!(answer(&f).is_ok(), "the room stayed open");
}

#[test]
fn no_floor_act_or_other_admin_act_is_a_wire_act() {
    let g = gateway();
    for act in [
        "cap",
        "chain",
        "scrub-add",
        "scrub-remove",
        "pause",
        "resume",
        "librarian",
        "reviewer",
        "admin-agent",
        "seal-label",
        "canary",
        "issue-credential",
        "nonsense",
    ] {
        let f = call(
            &g,
            "ada",
            "role_act",
            json!({"act":act,"label":"internal","name":"x","term":"y","room":"r","member":"alice"}),
        );
        assert_eq!(answer(&f), Err("unknown_act".into()), "{act}");
    }
    // nothing was attempted, so the admin agent has no refusal on record for any of them
    let f = call(&g, "ada", "role_act", json!({"act":"refusals"}));
    assert!(
        answer(&f).unwrap()["records"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["rule"] != "floors"),
        "a floor act reached the gateway"
    );
}

#[test]
fn a_role_act_needs_its_parameters() {
    let g = gateway();
    let f = call(&g, "ada", "role_act", json!({"act":"close"}));
    assert_eq!(answer(&f), Err("invalid_params".into()));
    let f = call(&g, "ada", "role_act", json!({}));
    assert_eq!(answer(&f), Err("invalid_params".into()));
}

#[test]
fn the_reviewer_reads_an_item_and_records_a_verdict_over_the_wire() {
    let g = gateway();
    let f = call(&g, "rev", "review_next", json!({}));
    let item = answer(&f).unwrap()["item"].clone();
    assert_eq!(item["text"], "candidate text");
    assert_eq!(item["room"], "r");
    let id = item["id"].as_i64().unwrap();
    let f = call(
        &g,
        "rev",
        "review_verdict",
        json!({"id":id,"verdict":"deny","reason":"leaks"}),
    );
    assert!(answer(&f).is_ok(), "{f:?}");
    let f = call(&g, "rev", "review_next", json!({}));
    assert!(answer(&f).unwrap()["item"].is_null());
}

#[test]
fn a_redact_verdict_carries_its_text_and_a_bad_verdict_is_invalid() {
    let g = gateway();
    let id = call(&g, "rev", "review_next", json!({}));
    let id = answer(&id).unwrap()["item"]["id"].as_i64().unwrap();
    let f = call(
        &g,
        "rev",
        "review_verdict",
        json!({"id":id,"verdict":"redact"}),
    );
    assert_eq!(
        answer(&f),
        Err("invalid_params".into()),
        "redact needs text"
    );
    let f = call(
        &g,
        "rev",
        "review_verdict",
        json!({"id":id,"verdict":"defer"}),
    );
    assert_eq!(answer(&f), Err("invalid_params".into()));
    let f = call(
        &g,
        "rev",
        "review_verdict",
        json!({"id":id,"verdict":"redact","text":"safe"}),
    );
    assert!(answer(&f).is_ok(), "{f:?}");
}

#[test]
fn a_member_that_is_not_the_reviewer_is_refused_both_review_methods() {
    let g = gateway();
    for who in ["alice", "ada"] {
        let f = call(&g, who, "review_next", json!({}));
        assert_eq!(answer(&f), Err("refused".into()), "{who}");
        assert_eq!(rule_of(&f).as_deref(), Some("reviewer"));
        let f = call(&g, who, "review_verdict", json!({"id":1,"verdict":"allow"}));
        assert_eq!(answer(&f), Err("refused".into()), "{who}");
    }
}

#[test]
fn the_reviewer_is_refused_a_role_act() {
    let g = gateway();
    let f = call(&g, "rev", "role_act", json!({"act":"close","room":"r"}));
    assert_eq!(answer(&f), Err("refused".into()));
    assert_eq!(rule_of(&f).as_deref(), Some("reviewer"));
}

#[test]
fn a_crossing_item_names_its_target_room_over_the_wire_and_never_its_requester() {
    let g = gateway();
    // the plain item first: it has no target room
    let f = call(&g, "rev", "review_next", json!({}));
    let plain = answer(&f).unwrap()["item"].clone();
    assert_eq!(plain["to_room"], "");
    let id = plain["id"].as_i64().unwrap();
    call(
        &g,
        "rev",
        "review_verdict",
        json!({"id":id,"verdict":"deny"}),
    );
    g.store_for_tests()
        .enqueue_review(
            &NewReview::new("r", 1, "public", "crossing").crossing("open", "alice"),
            1_002,
        )
        .unwrap();
    let f = call(&g, "rev", "review_next", json!({}));
    let item = answer(&f).unwrap()["item"].clone();
    assert_eq!(item["to_room"], "open");
    assert!(
        item.get("requester").is_none() && !item.to_string().contains("alice"),
        "the reviewer is not told who asked: {item}"
    );
}

#[test]
fn the_target_room_a_reviewer_reads_is_plain_characters_and_capped() {
    use super::role_methods::plain_room;
    assert_eq!(plain_room("open-room_1.x"), "open-room_1.x");
    assert_eq!(plain_room("a b\nc/\u{202e}d"), "abcd");
    assert_eq!(plain_room(&"x".repeat(100)).len(), 40);
}

#[test]
fn the_wire_item_carries_the_target_room_through_plain_room_only() {
    use super::role_methods::item_json;
    let item = klatch_item("a b\u{202e}/open");
    let v = item_json(&item);
    assert_eq!(v["to_room"], "abopen".to_string());
    let capped = item_json(&klatch_item(&"y".repeat(100)));
    assert_eq!(capped["to_room"].as_str().unwrap().len(), 40);
}

fn klatch_item(to_room: &str) -> crate::domain::ReviewItem {
    crate::domain::ReviewItem {
        id: 1,
        room: "r".into(),
        seq: 1,
        to_label: "public".into(),
        from_label: String::new(),
        reason: "crossing".into(),
        text: "t".into(),
        to_room: to_room.into(),
    }
}

#[test]
fn an_unknown_wire_act_is_named_capped_and_ascii() {
    let g = gateway();
    let hostile = format!(
        "\u{1b}[31m\u{202e}\u{e0041}\n{}\u{1b}]0;x\u{7}\r",
        "a".repeat(100 * 1024)
    );
    let f = call(&g, "ada", "role_act", json!({ "act": hostile }));
    let text = format!("{f:?}");
    let ServerFrame::Response { error: Some(e), .. } = &f[1] else {
        panic!("not an error: {text:.200}")
    };
    assert_eq!(e.code, "unknown_act");
    assert!(
        e.message.is_ascii() && !e.message.chars().any(char::is_control),
        "{:?}",
        e.message
    );
    assert!(e.message.len() < 400, "{}", e.message.len());
    let f = call(&g, "ada", "role_act", json!({ "act": "tele port" }));
    let ServerFrame::Response { error: Some(e), .. } = &f[1] else {
        panic!("not an error")
    };
    assert_eq!(
        e.message,
        "tele port is not an act a role can call over the wire"
    );
}
