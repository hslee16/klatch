//! #301: a member's kind (session, bot, service, human) is shown by `who` and on the board. A session's kind comes
//! from the session record klatch holds, never from a claim; a bot, a service or a human declares its own. The kind
//! is display only: it changes no rule and grants nothing.

use super::*;
use crate::domain::MemberKind;

fn kind_in_who(g: &Gateway<SpyStore>, who: &Principal, name: &str) -> Option<MemberKind> {
    g.who(who, "lobby")
        .unwrap()
        .into_iter()
        .find(|m| m.name == name)
        .unwrap()
        .kind
}

fn kind_on_board(g: &Gateway<SpyStore>, who: &Principal, name: &str) -> Option<MemberKind> {
    g.board(who, "lobby")
        .unwrap()
        .members
        .into_iter()
        .find(|m| m.name == name)
        .unwrap()
        .kind
}

fn lobby() -> Gateway<SpyStore> {
    let g = open();
    g.join(&alice(), "lobby").unwrap();
    g.join(&bob(), "lobby").unwrap();
    g
}

#[test]
fn a_member_that_declared_nothing_has_no_kind() {
    let g = lobby();
    assert_eq!(kind_in_who(&g, &alice(), "bob"), None);
    assert_eq!(kind_on_board(&g, &alice(), "bob"), None);
}

#[test]
fn a_declared_kind_shows_in_who_and_on_the_board() {
    let g = lobby();
    g.set_kind(&bob(), Some(MemberKind::Bot)).unwrap();
    assert_eq!(kind_in_who(&g, &alice(), "bob"), Some(MemberKind::Bot));
    assert_eq!(kind_on_board(&g, &alice(), "bob"), Some(MemberKind::Bot));
    assert_eq!(kind_in_who(&g, &alice(), "alice"), None);
    assert_eq!(g.kind_of(&bob()).unwrap(), Some(MemberKind::Bot));
}

#[test]
fn a_kind_is_replaced_and_removed_by_its_own_member() {
    let g = lobby();
    g.set_kind(&bob(), Some(MemberKind::Bot)).unwrap();
    g.set_kind(&bob(), Some(MemberKind::Human)).unwrap();
    assert_eq!(kind_in_who(&g, &alice(), "bob"), Some(MemberKind::Human));
    g.set_kind(&bob(), None).unwrap();
    assert_eq!(kind_in_who(&g, &alice(), "bob"), None);
    assert_eq!(g.store_for_tests().policy_get("kind:bob").unwrap(), None);
}

#[test]
fn a_member_can_declare_only_its_own_kind() {
    let g = lobby();
    g.set_kind(&alice(), Some(MemberKind::Service)).unwrap();
    assert_eq!(
        g.store_for_tests().policy_get("kind:alice").unwrap(),
        Some("service".into())
    );
    assert_eq!(g.store_for_tests().policy_get("kind:bob").unwrap(), None);
}

#[test]
fn a_session_record_makes_a_session_and_a_member_cannot_declare_one() {
    let g = lobby();
    assert!(is_invalid(g.set_kind(&bob(), Some(MemberKind::Session))));
    assert_eq!(kind_in_who(&g, &alice(), "bob"), None);
    g.note_session(&bob(), "sess-bob-1").unwrap();
    assert_eq!(kind_in_who(&g, &alice(), "bob"), Some(MemberKind::Session));
    assert_eq!(
        kind_on_board(&g, &alice(), "bob"),
        Some(MemberKind::Session)
    );
}

#[test]
fn what_klatch_knows_wins_over_a_declaration() {
    let g = lobby();
    g.set_kind(&bob(), Some(MemberKind::Human)).unwrap();
    g.note_session(&bob(), "sess-bob-1").unwrap();
    assert_eq!(kind_in_who(&g, &alice(), "bob"), Some(MemberKind::Session));
}

#[test]
fn a_stored_value_outside_the_vocabulary_reads_as_no_kind() {
    let g = lobby();
    g.store_for_tests()
        .policy_set("kind:bob", Some("admin"), 0)
        .unwrap();
    assert_eq!(kind_in_who(&g, &alice(), "bob"), None);
    assert_eq!(g.kind_of(&bob()).unwrap(), None);
}

#[test]
fn a_kind_grants_nothing() {
    let g = lobby().for_members();
    for k in [MemberKind::Bot, MemberKind::Service, MemberKind::Human] {
        g.set_kind(&bob(), Some(k)).unwrap();
        assert!(
            is_refused(g.admin_set_lease_cap(&bob(), Some(60))),
            "{k:?} must not act as the human"
        );
    }
    g.post(&bob(), "lobby", "an ordinary post").unwrap();
}
