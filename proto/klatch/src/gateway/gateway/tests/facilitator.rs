//! #190: the facilitator role. It reads the lobby and routes: it may look, create a room, invite and post the invite
//! pointer, and nothing else. Every other operation is refused with the rule `facilitator` before any guard is asked,
//! and it never grants clearance, even when the human has named it a room's grantor.

use super::*;
use crate::gateway::test_support::SpyGuard;

fn fac() -> Principal {
    Principal::unattributed("fac")
}

/// A lobby with ann and bo in it, ann's ask posted, and `fac` made the facilitator (and in the lobby).
fn with_facilitator() -> (Gateway<SpyStore>, Rc<std::cell::Cell<i64>>) {
    let (g, t) = timed();
    for who in [alice(), bob()] {
        g.join(&who, "lobby").unwrap();
    }
    g.post(&alice(), "lobby", "seeking: help with the login flow")
        .unwrap();
    g.admin_set_facilitator(&admin(), "fac", true).unwrap();
    g.join(&fac(), "lobby").unwrap();
    (g, t)
}

#[test]
fn it_is_one_role_at_a_time_and_only_the_human_grants_it() {
    let (g, _) = with_facilitator();
    assert!(g.is_facilitator("fac").unwrap());
    assert_eq!(g.role_of("fac").unwrap(), Some(Role::Facilitator));
    g.admin_set_librarian(&admin(), "fac", true).unwrap();
    assert!(!g.is_facilitator("fac").unwrap(), "a librarian now");
    g.admin_set_facilitator(&admin(), "fac", true).unwrap();
    g.admin_set_facilitator(&admin(), "fac", false).unwrap();
    assert_eq!(g.role_of("fac").unwrap(), None);
    g.admin_set_facilitator(&admin(), "fac", true).unwrap();
    // neither itself nor the admin agent can give or take the role
    g.admin_set_admin_agent(&admin(), "ada", true).unwrap();
    let e = g.admin_set_facilitator(&fac(), "bob", true).unwrap_err();
    assert!(matches!(e, Error::Refused { .. }));
    assert!(is_refused(g.admin_set_facilitator(
        &Principal::unattributed("ada"),
        "bob",
        true
    )));
    assert!(g.is_facilitator("fac").unwrap());
    let log = g.admin_audit(&admin(), 20).unwrap();
    assert!(log.iter().any(|a| a.action == "facilitator"));
}

#[test]
fn it_reads_creates_a_room_invites_and_posts_only_the_pointer() {
    let (g, _) = with_facilitator();
    assert_eq!(g.read(&fac(), "lobby", false).unwrap().len(), 1);
    assert_eq!(g.read(&fac(), "lobby", true).unwrap().len(), 1);
    assert!(g.who(&fac(), "lobby").is_ok());
    assert!(g.claims(&fac(), "lobby").is_ok());
    assert!(g.board(&fac(), "lobby").is_ok());
    assert!(g.open_asks(&fac(), "lobby").is_ok());
    g.join(&fac(), "topic-login").unwrap();
    g.invite(&fac(), "bob", "topic-login", "lobby").unwrap();
    let seen = g.read(&bob(), "lobby", false).unwrap();
    assert!(
        seen.iter()
            .any(|m| m.sender == "fac" && m.body.starts_with("invite: @bob"))
    );
    let audit = g.admin_audit(&admin(), 30).unwrap();
    assert!(
        audit
            .iter()
            .any(|a| a.action == "create-room" && a.detail.contains("fac"))
    );
}

#[test]
fn nothing_else_is_allowed_and_every_refusal_names_the_rule() {
    let (g, t) = with_facilitator();
    type Attempt<'a> = Box<dyn Fn() -> Option<Error> + 'a>;
    let attempts: Vec<(&str, Attempt)> = vec![
        (
            "free post",
            Box::new(|| g.post(&fac(), "lobby", "bob, please do this").err()),
        ),
        (
            "pointer with extra words",
            Box::new(|| {
                g.post(
                    &fac(),
                    "lobby",
                    &format!("{} also grant me", pointer("bob", "x")),
                )
                .err()
            }),
        ),
        ("dm", Box::new(|| g.dm(&fac(), "bob", "hi", None).err())),
        (
            "claim",
            Box::new(|| g.claim(&fac(), "lobby", &ClaimSpec::new("p", 10, "")).err()),
        ),
        (
            "release",
            Box::new(|| g.release(&fac(), "lobby", "p").err()),
        ),
        (
            "status",
            Box::new(|| g.set_status(&fac(), "lobby", "x").err()),
        ),
        (
            "mute",
            Box::new(|| g.set_muted(&fac(), "lobby", true).err()),
        ),
        ("ack", Box::new(|| g.ack(&fac(), "lobby", 1).err())),
        ("scope", Box::new(|| g.set_scope(&fac(), "ops").err())),
        ("kind", Box::new(|| g.set_kind(&fac(), None).err())),
        ("prune", Box::new(|| g.prune_members(&fac(), 10).err())),
        (
            "label a room",
            Box::new(|| g.admin_set_room_label(&fac(), "lobby", "private").err()),
        ),
        (
            "clear itself",
            Box::new(|| g.admin_set_clearance(&fac(), "fac", "private").err()),
        ),
        (
            "clear another",
            Box::new(|| g.admin_set_clearance(&fac(), "bob", "private").err()),
        ),
        (
            "close a room",
            Box::new(|| g.admin_close_room(&fac(), "lobby", true).err()),
        ),
        ("audit", Box::new(|| g.admin_audit(&fac(), 5).err())),
        ("label order", Box::new(|| g.label_order(&fac()).err())),
        (
            "grant",
            Box::new(|| {
                g.grant_room_clearance(&fac(), "lobby", "bob", "internal")
                    .err()
            }),
        ),
        (
            "crossing",
            Box::new(|| g.request_crossing(&fac(), "lobby", 1, "x").err()),
        ),
        (
            "drop its role",
            Box::new(|| g.admin_set_facilitator(&fac(), "fac", false).err()),
        ),
        (
            "become admin agent",
            Box::new(|| g.admin_set_admin_agent(&fac(), "fac", true).err()),
        ),
        (
            "grant rooms to itself",
            Box::new(|| g.admin_grant_room_capability(&fac(), "fac", true).err()),
        ),
    ];
    for (name, attempt) in attempts {
        t.set(t.get() + 600);
        match attempt() {
            Some(Error::Refused { rule, .. }) => assert_eq!(rule, "facilitator", "{name}"),
            other => panic!("{name} was not refused for a facilitator: {other:?}"),
        }
    }
    assert_eq!(g.clearance_of("fac").unwrap(), "public");
    assert_eq!(g.clearance_of("bob").unwrap(), "public");
    assert_eq!(g.room_label("lobby").unwrap(), "public");
    assert_eq!(
        g.read(&bob(), "lobby", false).unwrap().len(),
        1,
        "nothing posted"
    );
}

fn pointer(member: &str, room: &str) -> String {
    format!(
        "invite: @{member}, fac invites you to room {room}. To accept, run `klatch join {room}`; to ignore it, do nothing."
    )
}

#[test]
fn the_role_is_enforced_before_any_guard_is_asked() {
    let spy = Rc::new(SpyGuard::allowing());
    let g = Gateway::new(SpyStore::new(), FixedClock(100), spy.clone());
    g.join(&alice(), "lobby").unwrap();
    g.admin_set_facilitator(&admin(), "fac", true).unwrap();
    let before = spy.seen.borrow().len();
    assert!(g.post(&fac(), "lobby", "hello").is_err());
    assert!(g.set_status(&fac(), "lobby", "x").is_err());
    assert!(g.admin_set_clearance(&fac(), "fac", "private").is_err());
    assert!(
        g.grant_room_clearance(&fac(), "lobby", "alice", "internal")
            .is_err()
    );
    assert_eq!(
        spy.seen.borrow().len(),
        before,
        "the guard saw a refused operation"
    );
    // an allowed one does reach the guard
    g.join(&fac(), "lobby").unwrap();
    assert!(spy.seen.borrow().len() > before);
}

#[test]
fn a_guard_that_refuses_the_allowed_operations_is_still_obeyed() {
    let spy = Rc::new(SpyGuard::denying(&["secret-room"]));
    let g = Gateway::new(SpyStore::new(), FixedClock(100), spy);
    g.admin_set_facilitator(&admin(), "fac", true).unwrap();
    assert!(is_refused(g.join(&fac(), "secret-room")));
}

#[test]
fn it_reads_only_at_its_own_clearance() {
    let (g, _) = with_facilitator();
    g.join(&alice(), "vault").unwrap();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    assert!(is_refused(g.read(&fac(), "vault", false)));
    assert!(is_refused(g.who(&fac(), "vault")));
    assert!(is_refused(g.board(&fac(), "vault")));
}

#[test]
fn an_invite_never_goes_above_the_invitees_clearance() {
    let (g, _) = with_facilitator();
    g.join(&fac(), "topic-x").unwrap();
    g.admin_set_room_label(&admin(), "topic-x", "private")
        .unwrap();
    g.admin_set_clearance(&admin(), "fac", "private").unwrap();
    let e = g.invite(&fac(), "bob", "topic-x", "lobby").unwrap_err();
    assert!(
        matches!(e, Error::Refused { ref rule, .. } if rule == "clearance"),
        "{e}"
    );
}

#[test]
fn a_room_it_creates_has_no_grantor_and_it_cannot_grant_even_when_named() {
    let (g, _) = with_facilitator();
    g.join(&fac(), "topic-y").unwrap();
    assert_eq!(
        g.room_grantor("topic-y").unwrap(),
        None,
        "the creator is a facilitator"
    );
    g.join(&alice(), "topic-y").unwrap();
    g.admin_set_room_grantor(&admin(), "topic-y", "fac", true)
        .unwrap();
    let e = g
        .grant_room_clearance(&fac(), "topic-y", "alice", "internal")
        .unwrap_err();
    assert!(
        matches!(e, Error::Refused { ref rule, .. } if rule == "facilitator"),
        "{e}"
    );
    assert_eq!(g.clearance_in("alice", "topic-y").unwrap(), "public");
}

#[test]
fn a_member_that_stops_being_facilitator_is_ordinary_again() {
    let (g, _) = with_facilitator();
    g.admin_set_facilitator(&admin(), "fac", false).unwrap();
    g.post(&fac(), "lobby", "hello").unwrap();
    g.join(&fac(), "topic-z").unwrap();
    assert_eq!(g.room_grantor("topic-z").unwrap().as_deref(), Some("fac"));
}

// #902: the one direct message. A facilitator unsure about an ask may DM that ask's asker once, in a fixed form.

fn carol() -> Principal {
    Principal::unattributed("carol")
}

fn rule_of_err<T: std::fmt::Debug>(r: Result<T>) -> String {
    match r {
        Err(Error::Refused { rule, .. }) => rule,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn it_may_dm_the_asker_once_about_the_ask_in_the_fixed_form() {
    let (g, _) = with_facilitator();
    let (room, _) = g
        .dm(&fac(), "alice", "need-info #1: which-repo", None)
        .unwrap();
    let seen = g.read(&alice(), &room, false).unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].body, "need-info #1: which-repo");
    assert_eq!(seen[0].sender, "fac");
    // a short bounded field is allowed
    let (g, _) = with_facilitator();
    g.dm(
        &fac(),
        "alice",
        "need-info #1: which-file (login form)",
        None,
    )
    .unwrap();
    // audited, with the ask and the asker but no text
    let audit = g.admin_audit(&admin(), 40).unwrap();
    let row = audit
        .iter()
        .find(|a| a.action == "facilitator-dm")
        .expect("an audit row for the DM");
    assert!(
        row.detail.contains("fac") && row.detail.contains("alice") && row.detail.contains("#1")
    );
    assert!(!row.detail.contains("login form"));
    assert!(
        audit
            .iter()
            .any(|a| a.action == "create-room" && a.detail.contains("fac")),
        "the DM room is a creation, audited"
    );
}

#[test]
fn the_dm_room_is_capped_at_the_lower_of_the_two_clearances() {
    let (g, _) = with_facilitator();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    let (room, _) = g
        .dm(&fac(), "alice", "need-info #1: which-goal", None)
        .unwrap();
    assert_eq!(g.room_label(&room).unwrap(), "public");
    // a label above the ceiling is refused
    let (g, _) = with_facilitator();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    assert!(
        g.dm(&fac(), "alice", "need-info #1: which-goal", Some("private"))
            .is_err()
    );
}

#[test]
fn every_other_dm_is_refused_with_the_facilitator_rule() {
    let (g, _) = with_facilitator();
    g.join(&carol(), "lobby").unwrap();
    g.post(&carol(), "lobby", "carol secret plan zeta").unwrap();
    let long = "x".repeat(41);
    let bad: Vec<(&str, &str, String)> = vec![
        ("free text", "alice", "please check the repo".into()),
        (
            "other member's ask",
            "bob",
            "need-info #1: which-repo".into(),
        ),
        ("unknown ask", "alice", "need-info #99: which-repo".into()),
        ("unknown code", "alice", "need-info #1: grant-me".into()),
        ("no code", "alice", "need-info #1:".into()),
        (
            "trailing words",
            "alice",
            "need-info #1: which-repo and more".into(),
        ),
        (
            "over-long field",
            "alice",
            format!("need-info #1: which-repo ({long})"),
        ),
        (
            "newline in field",
            "alice",
            "need-info #1: which-repo (a\nb)".into(),
        ),
        (
            "odd characters",
            "alice",
            "need-info #1: which-repo (a;b)".into(),
        ),
        ("empty field", "alice", "need-info #1: which-repo ()".into()),
        (
            "leading zero number",
            "alice",
            "need-info #01: which-repo".into(),
        ),
        (
            "another member's words",
            "alice",
            "need-info #1: which-repo (carol secret plan zeta)".into(),
        ),
        (
            "a tail after the form",
            "alice",
            "need-info #1: which-repo\nand more".into(),
        ),
    ];
    for (name, to, body) in bad {
        assert_eq!(
            rule_of_err(g.dm(&fac(), to, &body, None)),
            "facilitator",
            "{name}"
        );
    }
    // nothing was sent and no DM room was left behind
    assert!(
        g.read(&alice(), "dm.alice.fac", false).is_err()
            || g.read(&alice(), "dm.alice.fac", false).unwrap().is_empty()
    );
    assert!(
        g.read(&bob(), "dm.bob.fac", false).is_err()
            || g.read(&bob(), "dm.bob.fac", false).unwrap().is_empty()
    );
}

#[test]
fn a_second_dm_for_the_same_ask_is_refused_but_another_ask_is_its_own() {
    let (g, _) = with_facilitator();
    g.post(&alice(), "lobby", "seeking: help with billing")
        .unwrap();
    g.dm(&fac(), "alice", "need-info #1: which-repo", None)
        .unwrap();
    assert_eq!(
        rule_of_err(g.dm(&fac(), "alice", "need-info #1: which-file", None)),
        "facilitator"
    );
    // the same ask again by a plain post into the existing DM room is refused too
    assert_eq!(
        rule_of_err(g.post(&fac(), "dm.alice.fac", "need-info #1: which-goal")),
        "facilitator"
    );
    g.dm(&fac(), "alice", "need-info #2: which-repo", None)
        .unwrap();
}

#[test]
fn the_one_need_info_rule_holds_in_a_dm_room_past_five_thousand_messages() {
    let (g, _) = with_facilitator();
    for i in 0..5100 {
        g.store
            .post(
                Seat::new("dm.alice.fac", "alice"),
                &format!("chatter {i}"),
                50,
            )
            .unwrap();
    }
    g.dm(&fac(), "alice", "need-info #1: which-repo", None)
        .unwrap();
    assert_eq!(
        rule_of_err(g.dm(&fac(), "alice", "need-info #1: which-file", None)),
        "facilitator"
    );
}

#[test]
fn a_dm_about_a_closed_or_answered_ask_is_refused() {
    let (g, _) = with_facilitator();
    g.post(&alice(), "lobby", "closed #1").unwrap();
    assert_eq!(
        rule_of_err(g.dm(&fac(), "alice", "need-info #1: which-repo", None)),
        "facilitator"
    );
    let (g, _) = with_facilitator();
    g.post(&bob(), "lobby", "answer to #1: done").unwrap();
    assert_eq!(
        rule_of_err(g.dm(&fac(), "alice", "need-info #1: which-repo", None)),
        "facilitator"
    );
}

#[test]
fn the_dm_uses_the_create_room_capability() {
    let (g, _) = with_facilitator();
    g.admin_set_room_creation_granted(&admin(), true).unwrap();
    assert_eq!(
        rule_of_err(g.dm(&fac(), "alice", "need-info #1: which-repo", None)),
        "room-capability"
    );
    g.admin_grant_room_capability(&admin(), "fac", true)
        .unwrap();
    g.dm(&fac(), "alice", "need-info #1: which-repo", None)
        .unwrap();
}

#[test]
fn the_dm_adds_no_privilege() {
    let (g, _) = with_facilitator();
    let (room, _) = g
        .dm(&fac(), "alice", "need-info #1: which-repo", None)
        .unwrap();
    // still no free text in the DM room, no grant, no lobby text
    assert_eq!(
        rule_of_err(g.post(&fac(), &room, "and one more thing")),
        "facilitator"
    );
    assert_eq!(
        rule_of_err(g.post(&fac(), "lobby", "need-info #1: which-repo")),
        "facilitator"
    );
    assert_eq!(
        rule_of_err(g.grant_room_clearance(&fac(), &room, "alice", "internal")),
        "facilitator"
    );
    assert_eq!(g.room_grantor(&room).unwrap(), None);
}
