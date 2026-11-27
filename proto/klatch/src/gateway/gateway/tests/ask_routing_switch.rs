//! #1460: the human's switch for ask routing (#1407). `admin_set_room_ask_routing` is the only way to set the policy key
//! `room:ROOM:ask-routing`: the human's alone, audited, only the three words `on`, `off` and `default`, only for a valid
//! room name, and it takes effect on the next ask.

use super::*;

const ASK: &str = "seeking: a retry limit";

fn who(name: &str) -> Principal {
    Principal::unattributed(name)
}

fn room_with_bob() -> Gateway<SpyStore> {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.join(&who("bob"), "r").unwrap();
    g
}

fn addressed(g: &Gateway<SpyStore>, seq: i64) -> usize {
    g.ask_routes(&alice(), "r")
        .unwrap()
        .iter()
        .filter(|r| r.seq == seq)
        .count()
}

fn key(g: &Gateway<SpyStore>) -> Option<String> {
    g.store_for_tests()
        .policy_get("room:r:ask-routing")
        .unwrap()
}

#[test]
fn off_stops_routing_on_the_next_ask_and_default_restores_it() {
    let g = room_with_bob();
    assert!(addressed(&g, g.post(&alice(), "r", "seeking: one").unwrap().seq) > 0);

    g.admin_set_room_ask_routing(&admin(), "r", Some(false))
        .unwrap();
    assert_eq!(key(&g), Some("off".into()));
    let off_seq = g.post(&alice(), "r", "seeking: two").unwrap().seq;
    assert_eq!(addressed(&g, off_seq), 0);

    g.admin_set_room_ask_routing(&admin(), "r", None).unwrap();
    assert_eq!(key(&g), None);
    let back = g.post(&alice(), "r", "seeking: three").unwrap().seq;
    assert!(addressed(&g, back) > 0);
}

#[test]
fn on_is_stored_and_routes_like_the_default() {
    let g = room_with_bob();
    g.admin_set_room_ask_routing(&admin(), "r", Some(true))
        .unwrap();
    assert_eq!(key(&g), Some("on".into()));
    assert!(addressed(&g, g.post(&alice(), "r", ASK).unwrap().seq) > 0);
}

#[test]
fn the_switch_is_per_room() {
    let g = room_with_bob();
    g.admin_set_room_ask_routing(&admin(), "other", Some(false))
        .unwrap();
    assert!(addressed(&g, g.post(&alice(), "r", ASK).unwrap().seq) > 0);
}

#[test]
fn a_bad_room_name_is_refused_and_stores_nothing() {
    let g = open();
    for bad in ["", "has space", "a:b", "new\nline", "r:ask-routing"] {
        assert!(
            is_invalid(g.admin_set_room_ask_routing(&admin(), bad, Some(false))),
            "{bad:?}"
        );
    }
    assert!(g.store_for_tests().policy_list("room:").unwrap().is_empty());
    assert!(g.admin_audit(&admin(), 10).unwrap().is_empty());
}

#[test]
fn an_admin_agent_and_a_member_may_not_set_it() {
    let g = open();
    g.admin_set_admin_agent(&admin(), "ada", true).unwrap();
    assert_eq!(
        rule_of(
            g.admin_set_room_ask_routing(&who("ada"), "r", Some(false))
                .unwrap_err()
        ),
        "admin-scope"
    );
    assert_eq!(key(&g), None);
    let members = open().for_members();
    assert!(is_refused(members.admin_set_room_ask_routing(
        &alice(),
        "r",
        Some(false)
    )));
}

#[test]
fn each_change_is_audited_with_the_room_and_one_of_three_words() {
    let g = open();
    for (v, word) in [(Some(false), "off"), (Some(true), "on"), (None, "default")] {
        g.admin_set_room_ask_routing(&admin(), "slow", v).unwrap();
        let rows = g.admin_audit(&admin(), 10).unwrap();
        assert!(
            rows.iter()
                .any(|r| r.action == "room-ask-routing" && r.target == format!("slow {word}")),
            "{word}: {rows:?}"
        );
    }
}

#[test]
fn a_hand_written_key_other_than_off_still_routes() {
    let g = room_with_bob();
    g.store_for_tests()
        .policy_set("room:r:ask-routing", Some("OFF "), 1)
        .unwrap();
    assert!(addressed(&g, g.post(&alice(), "r", ASK).unwrap().seq) > 0);
}
