//! `klatch could-answer` (#671, part 1): the read-only check a supervisor outside klatch asks before it starts a
//! helper, through the real binary.

mod common;
use common::*;

fn ask(h: &Home) -> String {
    h.ok("alice", &["join", "lobby"]);
    h.ok("alice", &["post", "lobby", "seeking: a thing"]);
    let out = h.ok("alice", &["asks", "lobby"]);
    out.strip_prefix('#')
        .and_then(|r| r.split(' ').next())
        .unwrap_or_else(|| panic!("no ask listed: {out}"))
        .to_string()
}

#[test]
fn a_member_checks_itself_and_gets_yes_for_an_open_ask() {
    let h = Home::new();
    let n = ask(&h);
    assert_eq!(h.ok("bob", &["could-answer", "lobby", &n]).trim(), "yes");
}

#[test]
fn the_asker_gets_no_with_the_rule_and_a_reason() {
    let h = Home::new();
    let n = ask(&h);
    let out = h.ok("alice", &["could-answer", "lobby", &n]);
    let mut lines = out.lines();
    assert_eq!(lines.next(), Some("no"));
    assert!(
        lines.next().unwrap().starts_with("own-ask: "),
        "unexpected: {out}"
    );
}

#[test]
fn an_ask_that_is_not_open_is_no_such_ask() {
    let h = Home::new();
    let n = ask(&h);
    let missing = (n.parse::<i64>().unwrap() + 40).to_string();
    let out = h.ok("bob", &["could-answer", "lobby", &missing]);
    assert!(out.starts_with("no\nno-such-ask: "), "unexpected: {out}");
}
