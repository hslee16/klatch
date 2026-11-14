use super::*;

#[test]
fn the_board_combines_asks_offers_members_and_status() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.store
        .post(Seat::new("r", "bob"), "seeking: slugify", 90)
        .unwrap();
    g.store
        .post(Seat::new("r", "alice"), "offering: helpers", 95)
        .unwrap();
    g.store
        .post(Seat::new("r", "bob"), "status: busy", 96)
        .unwrap();
    let b = g.board(&alice(), "r").unwrap();
    assert_eq!(b.room, "r");
    assert_eq!(b.asks.len(), 1);
    assert_eq!(b.offers.len(), 1);
    assert_eq!(b.offers[0].sender, "alice");
    let bob = b.members.iter().find(|m| m.name == "bob").unwrap();
    assert_eq!(bob.status.as_deref(), Some("busy"));
    let alice = b.members.iter().find(|m| m.name == "alice").unwrap();
    assert_eq!(alice.status, None);
}

#[test]
fn the_board_lists_recent_members_and_counts_the_long_gone() {
    let g = Gateway::new(SpyStore::new(), FixedClock(1000), AllowAll);
    g.join(&alice(), "r").unwrap();
    g.store.join("r", "ghost", 1).unwrap();
    g.store.join("r", "bob", 999).unwrap();
    let b = g.board(&alice(), "r").unwrap();
    let names: Vec<_> = b.members.iter().map(|m| m.name.as_str()).collect();
    assert!(
        names.contains(&"alice") && names.contains(&"bob"),
        "{names:?}"
    );
    assert!(
        !names.contains(&"ghost"),
        "listed a member not seen for ages: {names:?}"
    );
    assert_eq!(b.stale_members, 1);
}

#[test]
fn reading_the_board_does_not_make_the_reader_a_member() {
    let g = open();
    g.join(&bob(), "r").unwrap();
    g.board(&alice(), "r").unwrap();
    let names: Vec<_> = g
        .who(&bob(), "r")
        .unwrap()
        .into_iter()
        .map(|m| m.name)
        .collect();
    assert!(!names.contains(&"alice".to_string()), "{names:?}");
}

#[test]
fn a_status_goes_through_the_guard_and_shows_in_who() {
    let g = Gateway::new(
        SpyStore::new(),
        FixedClock(5),
        Rc::new(SpyGuard::allowing()),
    );
    g.join(&alice(), "r").unwrap();
    g.set_status(&alice(), "r", "done leaving").unwrap();
    let m = g.who(&alice(), "r").unwrap();
    assert_eq!(m[0].status.as_deref(), Some("done leaving"));
}

#[test]
fn the_board_prefers_the_status_fact_and_drops_the_offers_of_a_member_who_is_done() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.store
        .post(Seat::new("r", "bob"), "offering: helpers", 90)
        .unwrap();
    g.set_status(&bob(), "r", "done leaving").unwrap();
    let b = g.board(&alice(), "r").unwrap();
    let bob = b.members.iter().find(|m| m.name == "bob").unwrap();
    assert_eq!(bob.status.as_deref(), Some("done leaving"));
    assert!(
        b.offers.is_empty(),
        "a done member's offer stayed on the board"
    );
}

#[test]
fn an_offer_goes_when_its_sender_has_not_been_seen_for_a_while() {
    let g = Gateway::new(SpyStore::new(), FixedClock(1000), AllowAll);
    g.join(&alice(), "r").unwrap();
    g.store
        .post(Seat::new("r", "ghost"), "offering: old help", 1)
        .unwrap();
    g.store
        .post(Seat::new("r", "bob"), "offering: fresh help", 999)
        .unwrap();
    let b = g.board(&alice(), "r").unwrap();
    let who: Vec<_> = b.offers.iter().map(|o| o.sender.as_str()).collect();
    assert_eq!(who, ["bob"], "an offer outlived its sender's presence");
}

#[test]
fn a_present_member_keeps_its_offer_without_renewing_it() {
    let g = Gateway::new(SpyStore::new(), FixedClock(5000), AllowAll);
    g.join(&alice(), "r").unwrap();
    g.store
        .post(Seat::new("r", "bob"), "offering: help", 100)
        .unwrap();
    g.store.wait_poll("r", "bob", 4990).unwrap();
    let b = g.board(&alice(), "r").unwrap();
    assert_eq!(
        b.offers.len(),
        1,
        "a waiting member lost its offer to the clock"
    );
}
