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
