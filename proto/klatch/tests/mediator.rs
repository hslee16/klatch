//! `klatch stuck` and `klatch mediator` (#271), end to end through the binary, on the stuck cases recorded in
//! `fixtures/stuck-cases.json`. Each member has its own scope except where a case needs a conflict.
mod common;

use common::*;
use serde_json::Value;

fn cases() -> Vec<Value> {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/stuck-cases.json"
    ))
    .unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    v["cases"].as_array().unwrap().clone()
}

/// A lobby with the members of every case and run the case's steps.
fn play(case: &Value) -> Home {
    let h = Home::new();
    for n in ["mediator", "ann", "bo", "cy"] {
        h.ok(n, &["join", "lobby"]);
    }
    for step in case["steps"].as_array().unwrap() {
        let words: Vec<&str> = step
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w.as_str().unwrap())
            .collect();
        let (who, refused) = match words[0].strip_prefix('!') {
            Some(w) => (w, true),
            None => (words[0], false),
        };
        if refused {
            h.fail(who, &words[1..]);
        } else {
            h.ok(who, &words[1..]);
        }
    }
    h
}

fn kinds(h: &Home, who: &str) -> Vec<String> {
    h.ok(who, &["stuck", "lobby"])
        .lines()
        .map(|l| l.split(' ').next().unwrap().to_string())
        .collect()
}

#[test]
fn every_recorded_case_shows_the_events_it_expects() {
    for case in cases() {
        let h = play(&case);
        let want: Vec<String> = case["expect_kinds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k.as_str().unwrap().to_string())
            .collect();
        assert_eq!(kinds(&h, "cy"), want, "{}", case["name"]);
    }
}

#[test]
fn every_recorded_case_gets_the_proposals_it_expects_and_only_proposals() {
    for case in cases() {
        let h = play(&case);
        h.ok("mediator", &["mediator", "--once"]);
        let seen = h.ok("cy", &["read", "lobby"]);
        let n = seen.matches("mediator proposal").count();
        assert_eq!(n as u64, case["expect_proposals"].as_u64().unwrap(), "{}: {seen}", case["name"]);
    }
}

#[test]
fn the_undetectable_cases_are_marked_so_and_show_nothing() {
    let names: Vec<String> = cases()
        .iter()
        .filter(|c| c["detectable"] == false)
        .map(|c| c["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        names,
        ["overwrite-after-deadlock", "two-tasks-need-the-same-result"]
    );
}

#[test]
fn a_cycle_through_asks_gets_a_room_and_an_invite_pointer_for_each_member() {
    let h = play(&cases()[0]);
    let out = h.ok("mediator", &["mediator", "--once"]);
    assert!(out.contains("invited ann to mediate-1") && out.contains("invited bo to mediate-1"), "{out}");
    let bo = h.ok("bo", &["read", "lobby"]);
    assert!(bo.contains("klatch join mediate-1"), "{bo}");
}

#[test]
fn the_mediator_leaves_the_claims_exactly_as_they_were() {
    let h = play(&cases()[1]);
    let before = h.ok("cy", &["claims", "lobby"]);
    h.ok("mediator", &["mediator", "--once"]);
    assert_eq!(h.ok("cy", &["claims", "lobby"]), before);
}
