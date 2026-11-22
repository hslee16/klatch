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

#[test]
fn the_digest_counts_what_the_view_hides_with_the_newest_seq_and_no_content() {
    let g = lobby();
    g.set_delivery_view(&alice(), DeliveryView::Scope).unwrap();
    g.post(&bob(), "lobby", "same repository").unwrap();
    g.post(&carol(), "lobby", "secret chatter one").unwrap();
    g.post(&carol(), "lobby", "secret chatter two").unwrap();
    let d = g.digest(&alice()).unwrap();
    assert_eq!(d.rooms[0].outside_view, Some((2, 3)));
    let text = crate::domain::render_hook(&d.rooms, d.now);
    assert!(
        text.contains("2 messages outside your view in room lobby, newest #3"),
        "{text}"
    );
    assert!(!text.contains("secret chatter"), "{text}");
}

#[test]
fn a_member_on_the_all_view_or_with_nothing_hidden_gets_no_pointer() {
    let g = lobby();
    g.post(&carol(), "lobby", "chatter").unwrap();
    assert_eq!(g.digest(&alice()).unwrap().rooms[0].outside_view, None);
    g.set_delivery_view(&alice(), DeliveryView::Scope).unwrap();
    g.post(&bob(), "lobby", "same repository").unwrap();
    assert_eq!(g.digest(&alice()).unwrap().rooms[0].outside_view, None);
}

#[test]
fn the_wake_reports_the_same_hidden_count_as_the_digest() {
    let g = lobby();
    g.set_delivery_view(&alice(), DeliveryView::Scope).unwrap();
    g.post(&carol(), "lobby", "other repository chatter")
        .unwrap();
    g.post(&carol(), "lobby", "more chatter").unwrap();
    assert_eq!(g.wakeable(&alice(), "lobby").unwrap(), 0);
    assert_eq!(g.outside_view(&alice(), "lobby").unwrap(), Some((2, 2)));
    assert_eq!(
        g.digest(&alice()).unwrap().rooms[0].outside_view,
        Some((2, 2))
    );
}

#[test]
fn a_singular_hidden_message_reads_correctly() {
    let g = lobby();
    g.set_delivery_view(&alice(), DeliveryView::Scope).unwrap();
    g.post(&carol(), "lobby", "chatter").unwrap();
    let d = g.digest(&alice()).unwrap();
    let text = crate::domain::render_hook(&d.rooms, d.now);
    assert!(
        text.contains("1 message outside your view in room lobby, newest #1"),
        "{text}"
    );
}

fn broadcast_post(g: &Gateway<SpyStore>, who: &Principal, body: &str) {
    let h = crate::gateway::Handoff {
        kind: "broadcast",
        payload: None,
    };
    g.post_handoff(who, ("lobby", body, None), &h).unwrap();
}

// #674: a broadcast is a message kind, so a narrowed view passes it with no @all in the text
#[test]
fn a_narrowed_view_shows_a_message_of_kind_broadcast() {
    let g = lobby();
    g.set_delivery_view(&alice(), DeliveryView::Scope).unwrap();
    g.post(&carol(), "lobby", "other repository chatter")
        .unwrap();
    broadcast_post(&g, &carol(), "the build is red");
    assert_eq!(
        bodies(&g.digest(&alice()).unwrap()),
        vec!["the build is red"]
    );
}

// #674: the old word keeps working until clients move; a kind other than broadcast is no broadcast
#[test]
fn the_all_word_still_broadcasts_and_another_kind_does_not() {
    let g = lobby();
    g.set_delivery_view(&alice(), DeliveryView::Scope).unwrap();
    let h = crate::gateway::Handoff {
        kind: "work-order",
        payload: None,
    };
    g.post_handoff(&carol(), ("lobby", "some order", None), &h)
        .unwrap();
    g.post(&carol(), "lobby", "@all the old way").unwrap();
    assert_eq!(
        bodies(&g.digest(&alice()).unwrap()),
        vec!["@all the old way"]
    );
}

// #674: a busy digest keeps a kind broadcast before newer messages, like an @all one
#[test]
fn a_message_of_kind_broadcast_counts_as_addressed_in_a_busy_digest() {
    let g = lobby();
    broadcast_post(&g, &carol(), "heads up");
    g.post(&carol(), "lobby", "chatter").unwrap();
    let d = g.digest(&alice()).unwrap();
    let r = d.rooms.iter().find(|r| r.room == "lobby").unwrap();
    assert_eq!(r.addressed, vec![1]);
}
