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

fn who(name: &str) -> Principal {
    Principal::unattributed(name)
}

/// A members' gateway: roles are granted first through the terminal gateway (no `for_members`), as the human does.
fn gateway() -> Gateway<SqliteStore> {
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

fn call(g: &Gateway<SqliteStore>, member: &str, method: &str, params: Value) -> Vec<ServerFrame> {
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
fn answer(frames: &[ServerFrame]) -> Result<Value, String> {
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

fn rule_of(frames: &[ServerFrame]) -> Option<String> {
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
