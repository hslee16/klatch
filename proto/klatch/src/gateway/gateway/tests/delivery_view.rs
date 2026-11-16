//! #186: the delivery view. A member may narrow what hooks and the wake count show to its own scope (repository), what
//! is addressed to it, and broadcasts (`@all`). Narrowing never hides what is addressed to the member, and an explicit
//! `read` is never narrowed. The default view is everything, so nothing changes until a member narrows its own.

use super::*;
use crate::gateway::DeliveryView;

fn carol() -> Principal {
    Principal::unattributed("carol")
}

fn bodies(d: &Digest) -> Vec<String> {
    d.rooms
        .iter()
        .flat_map(|r| r.messages.iter().map(|m| m.body.clone()))
        .collect()
}

/// alice and bob work in repository `git:a`, carol in `git:b`; all three are in room `lobby`, and alice listens.
fn lobby() -> Gateway<SpyStore> {
    let g = open();
    g.set_scope(&alice(), "git:a").unwrap();
    g.set_scope(&bob(), "git:a").unwrap();
    g.set_scope(&carol(), "git:b").unwrap();
    for p in [alice(), bob(), carol()] {
        g.join(&p, "lobby").unwrap();
    }
    g
}

#[test]
fn by_default_a_member_is_shown_everything() {
    let g = lobby();
    g.post(&carol(), "lobby", "from the other repository")
        .unwrap();
    assert_eq!(bodies(&g.digest(&alice()).unwrap()).len(), 1);
    assert_eq!(g.delivery_view(&alice()).unwrap(), DeliveryView::All);
}

#[test]
fn a_narrowed_view_shows_own_scope_addressed_and_broadcast_messages() {
    let g = lobby();
    g.set_delivery_view(&alice(), DeliveryView::Scope).unwrap();
    g.post(&bob(), "lobby", "same repository").unwrap();
    g.post(&carol(), "lobby", "other repository chatter")
        .unwrap();
    g.post(&carol(), "lobby", "@alice a question for you")
        .unwrap();
    g.post(&carol(), "lobby", "@all the build is red").unwrap();
    let shown = bodies(&g.digest(&alice()).unwrap());
    assert_eq!(
        shown,
        vec![
            "same repository",
            "@alice a question for you",
            "@all the build is red"
        ]
    );
}

#[test]
fn narrowing_never_hides_a_message_addressed_to_the_member_even_from_a_muted_room_or_a_dm() {
    let g = lobby();
    g.set_delivery_view(&alice(), DeliveryView::Scope).unwrap();
    g.set_muted(&alice(), "lobby", true).unwrap();
    g.post(&carol(), "lobby", "@alice muted but addressed")
        .unwrap();
    g.dm(&carol(), "alice", "a direct message, no mention", None)
        .unwrap();
    let shown = bodies(&g.digest(&alice()).unwrap());
    assert!(shown.contains(&"@alice muted but addressed".to_string()));
    assert!(
        shown.contains(&"a direct message, no mention".to_string()),
        "{shown:?}"
    );
}

#[test]
fn a_narrowed_view_never_narrows_an_explicit_read() {
    let g = lobby();
    g.set_delivery_view(&alice(), DeliveryView::Scope).unwrap();
    g.post(&carol(), "lobby", "other repository chatter")
        .unwrap();
    assert!(bodies(&g.digest(&alice()).unwrap()).is_empty());
    assert_eq!(g.read(&alice(), "lobby", false).unwrap().len(), 1);
}

#[test]
fn the_wake_count_follows_the_view_too() {
    let g = lobby();
    g.set_delivery_view(&alice(), DeliveryView::Scope).unwrap();
    g.post(&carol(), "lobby", "other repository chatter")
        .unwrap();
    assert_eq!(g.wakeable(&alice(), "lobby").unwrap(), 0);
    g.post(&carol(), "lobby", "@alice now you").unwrap();
    assert_eq!(g.wakeable(&alice(), "lobby").unwrap(), 1);
}

#[test]
fn the_view_is_the_members_own_and_can_be_widened_again() {
    let g = lobby();
    g.set_delivery_view(&alice(), DeliveryView::Scope).unwrap();
    assert_eq!(g.delivery_view(&bob()).unwrap(), DeliveryView::All);
    g.set_delivery_view(&alice(), DeliveryView::All).unwrap();
    g.post(&carol(), "lobby", "other repository chatter")
        .unwrap();
    assert_eq!(bodies(&g.digest(&alice()).unwrap()).len(), 1);
}

#[test]
fn two_members_with_no_scope_share_one_and_see_each_other() {
    let g = open();
    for p in [alice(), bob()] {
        g.join(&p, "lobby").unwrap();
    }
    g.set_delivery_view(&alice(), DeliveryView::Scope).unwrap();
    g.post(&bob(), "lobby", "no scope either").unwrap();
    assert_eq!(bodies(&g.digest(&alice()).unwrap()).len(), 1);
}
