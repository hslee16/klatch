//! The `mute` wire method (#334): a member mutes and unmutes a room for itself, and its digest follows.

use super::*;
use crate::gateway::Digest;

fn bodies(d: &Digest) -> Vec<&str> {
    d.rooms
        .iter()
        .flat_map(|r| r.messages.iter().map(|m| m.body.as_str()))
        .collect()
}

fn chatty() -> Gateway<SqliteStore> {
    let g = gateway();
    g.join(&alice(), "r").unwrap();
    g.post(&bob(), "r", "chatter one").unwrap();
    g.post(&bob(), "r", "@alice please look at this").unwrap();
    g.post(&bob(), "r", "chatter two").unwrap();
    g
}

fn mute(
    g: &Gateway<SqliteStore>,
    who: &Principal,
    params: Value,
) -> (bool, Option<(String, String)>) {
    let f = run(g, who, &[req("m", "mute", params)]);
    let (ok, _, err) = response(&f, "m");
    (ok, err)
}

#[test]
fn muting_over_the_wire_narrows_the_digest_to_mentions() {
    let g = chatty();
    let (ok, _) = mute(&g, &alice(), json!({"room":"r","muted":true}));
    assert!(ok);
    assert_eq!(
        bodies(&g.digest(&alice()).unwrap()),
        vec!["@alice please look at this"]
    );
}

#[test]
fn the_mute_result_is_an_empty_object() {
    let g = chatty();
    let f = run(
        &g,
        &alice(),
        &[req("m", "mute", json!({"room":"r","muted":true}))],
    );
    assert_eq!(response(&f, "m").1, Some(json!({})));
}

#[test]
fn unmuting_over_the_wire_brings_everything_back() {
    let g = chatty();
    mute(&g, &alice(), json!({"room":"r","muted":true}));
    g.digest(&alice()).unwrap();
    let (ok, _) = mute(&g, &alice(), json!({"room":"r","muted":false}));
    assert!(ok);
    g.post(&bob(), "r", "chatter three").unwrap();
    assert_eq!(bodies(&g.digest(&alice()).unwrap()), vec!["chatter three"]);
}

#[test]
fn a_wire_mute_leaves_read_alone() {
    let g = chatty();
    mute(&g, &alice(), json!({"room":"r","muted":true}));
    let f = run(
        &g,
        &alice(),
        &[req("r", "read", json!({"room":"r","peek":true}))],
    );
    let n = response(&f, "r").1.unwrap()["messages"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(n, 3);
}

#[test]
fn a_wire_mute_is_the_callers_own_and_a_member_param_has_no_effect() {
    let g = chatty();
    g.join(&bob(), "r").unwrap();
    g.post(&alice(), "r", "from alice").unwrap();
    mute(
        &g,
        &alice(),
        json!({"room":"r","muted":true,"member":"bob","sender":"bob"}),
    );
    let bob_sees = bodies(&g.digest(&bob()).unwrap()).join("|");
    assert!(bob_sees.contains("from alice"), "{bob_sees}");
    let alice_sees = bodies(&g.digest(&alice()).unwrap()).join("|");
    assert!(!alice_sees.contains("chatter one"), "{alice_sees}");
}

#[test]
fn mute_needs_a_room_and_a_boolean() {
    let g = gateway();
    for params in [
        json!({}),
        json!({"room":"r"}),
        json!({"muted":true}),
        json!({"room":"r","muted":"yes"}),
        json!({"room":"r","muted":1}),
        json!({"room":5,"muted":true}),
    ] {
        let (ok, err) = mute(&g, &alice(), params.clone());
        assert!(!ok, "{params}");
        assert_eq!(err.unwrap().0, "invalid_params", "{params}");
    }
}

#[test]
fn muting_a_room_with_a_bad_name_is_invalid() {
    let g = gateway();
    let (ok, err) = mute(&g, &alice(), json!({"room":"bad room","muted":true}));
    assert!(!ok);
    assert_eq!(err.unwrap().0, "invalid");
}

struct DenyMute;
impl Guard for DenyMute {
    fn check(&self, _who: &Principal, op: &Operation) -> Result<()> {
        match op {
            Operation::Mute { .. } => Err(Error::Refused {
                rule: "test.deny-mute".into(),
                reason: "muting is not allowed".into(),
            }),
            _ => Ok(()),
        }
    }
}

#[test]
fn a_guard_that_refuses_a_mute_refuses_it_over_the_wire_too() {
    let g = Gateway::new(
        SqliteStore::open_in_memory().unwrap(),
        FixedClock(1),
        DenyMute,
    );
    g.join(&alice(), "r").unwrap();
    let f = run(
        &g,
        &alice(),
        &[req("m", "mute", json!({"room":"r","muted":true}))],
    );
    assert!(
        matches!(&f[1], ServerFrame::Refusal { request: Some(r), rule, .. } if r == "m" && rule == "test.deny-mute"),
        "{f:?}"
    );
    let (ok, _, err) = response(&f, "m");
    assert!(!ok);
    assert_eq!(err.unwrap().0, "refused");
}
