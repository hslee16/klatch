//! #251: a member's card, derived from the board and the live claims. The requester view is the asks it posted; the
//! helper view is the asks it serves and the claims it holds. Nothing is stored; a member that is not on the board
//! has no card; a card is a read like the board and refuses where the board does.

use super::*;

fn lobby() -> Gateway<SpyStore> {
    let (g, _) = timed();
    g.set_scope(&alice(), "git:a").unwrap();
    g.join(&alice(), "lobby").unwrap();
    g.join(&bob(), "lobby").unwrap();
    g
}

#[test]
fn a_member_that_is_in_the_room_has_a_card_with_its_scope() {
    let g = lobby();
    let c = g.member_card(&bob(), "lobby", "alice").unwrap().unwrap();
    assert_eq!(c.name, "alice");
    assert_eq!(c.scope.as_deref(), Some("git:a"));
    let b = g.member_card(&alice(), "lobby", "bob").unwrap().unwrap();
    assert_eq!(b.scope, None);
}

#[test]
fn a_name_that_is_not_on_the_board_has_no_card() {
    let g = lobby();
    assert_eq!(g.member_card(&alice(), "lobby", "nobody").unwrap(), None);
}

#[test]
fn a_bad_name_or_room_is_invalid() {
    let g = lobby();
    assert!(is_invalid(g.member_card(&alice(), "lobby", "bad name\n")));
    assert!(is_invalid(g.member_card(&alice(), "bad room", "alice")));
}

#[test]
fn the_requester_card_lists_its_asks_and_the_helper_card_lists_what_it_serves() {
    let g = lobby();
    g.post(&alice(), "lobby", "seeking: a slugify helper")
        .unwrap();
    g.claim(&bob(), "lobby", &ClaimSpec::new("ask:1", 90, ""))
        .unwrap();
    g.claim(&bob(), "lobby", &ClaimSpec::new("src/lib.rs", 600, "editing"))
        .unwrap();
    let a = g.member_card(&bob(), "lobby", "alice").unwrap().unwrap();
    assert_eq!(a.asks.len(), 1);
    assert!(a.asks[0].claimed);
    assert!(a.serving.is_empty());
    let b = g.member_card(&alice(), "lobby", "bob").unwrap().unwrap();
    assert!(b.asks.is_empty());
    assert_eq!(b.serving.len(), 1);
    assert_eq!(
        b.claims.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
        vec!["src/lib.rs"]
    );
}

#[test]
fn a_card_is_a_read_and_refuses_where_the_board_refuses() {
    let g = lobby();
    let guard = std::rc::Rc::new(crate::gateway::test_support::SpyGuard::denying(&["lobby"]));
    let denied = Gateway::new(SpyStore::new(), FixedClock(100), guard);
    assert!(is_refused(denied.member_card(&alice(), "lobby", "alice")));
    assert!(g.member_card(&alice(), "lobby", "alice").unwrap().is_some());
}
