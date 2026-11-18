//! `klatch router` (#189): the reference router as a command, end to end through the binary. Each member has its own
//! scope (a repository), so equal or overlapping claim paths do not conflict.
mod common;

use common::*;

fn lobby() -> Home {
    let h = Home::new();
    for (n, scope) in [
        ("router", "ops"),
        ("ann", "alpha"),
        ("bo", "beta"),
        ("cy", "gamma"),
    ] {
        h.ok(n, &["join", "lobby"]);
        h.ok(n, &["scope", scope]);
    }
    h.ok("ann", &["claim", "lobby", "src/auth"]);
    h.ok("bo", &["claim", "lobby", "src/auth/login.rs"]);
    h.ok("cy", &["claim", "lobby", "docs"]);
    h.ok(
        "ann",
        &["post", "lobby", "seeking: help with the login flow"],
    );
    h
}

#[test]
fn one_pass_invites_the_member_whose_claim_overlaps_the_askers_topic() {
    let h = lobby();
    let out = h.ok("router", &["router", "--once"]);
    assert!(
        out.contains("invited bo to topic-src-auth for ask #1"),
        "{out}"
    );
    let bo = h.ok("bo", &["read", "lobby"]);
    assert!(
        bo.contains("@bo") && bo.contains("klatch join topic-src-auth"),
        "{bo}"
    );
    assert!(!h.ok("cy", &["read", "lobby"]).contains("@cy"));
}

#[test]
fn a_second_run_does_not_invite_a_member_who_already_joined() {
    let h = lobby();
    h.ok("router", &["router", "--once"]);
    h.ok("bo", &["join", "topic-src-auth"]);
    assert_eq!(h.ok("router", &["router", "--once"]).trim(), "");
}

#[test]
fn a_rule_must_be_prefix_equals_member() {
    let h = lobby();
    let err = h.fail("router", &["router", "--once", "--rule", "nonsense"]);
    assert!(err.contains("prefix=member"), "{err}");
}
