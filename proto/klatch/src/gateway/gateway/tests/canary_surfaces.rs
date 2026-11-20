//! #954: every surface that returns room messages treats the canary kind the same way (#347): it is never shown or
//! counted outside the first digest, it is no message a member can point at, and the tripwire still finds its token.

use super::*;
use crate::gateway::{HistoryRange, Subscription};

/// `vault` is private with alice and bob cleared and joined; the canary is seq 1 and alice's "real" is seq 2. A public
/// room `open` that alice joined stands below it.
fn world() -> (Gateway<SpyStore>, String) {
    let g = open();
    for p in ["alice", "bob"] {
        g.admin_set_clearance(&admin(), p, "private").unwrap();
    }
    g.join(&alice(), "vault").unwrap();
    g.join(&bob(), "vault").unwrap();
    g.join(&alice(), "open").unwrap();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    let token = g
        .store
        .policy_list("canary:")
        .unwrap()
        .pop()
        .expect("planted")
        .0
        .trim_start_matches("canary:")
        .to_string();
    g.post(&alice(), "vault", "real").unwrap();
    (g, token)
}

fn bodies(ms: &[Message]) -> Vec<String> {
    ms.iter().map(|m| m.body.clone()).collect()
}

#[test]
fn a_subscription_replays_past_the_canary_and_announces_caught_up_only_at_the_head() {
    let (g, token) = world();
    // one message per poll: the page that holds only the canary must not look like the end of the room
    let mut sub = Subscription::new("vault", 0, 16);
    let mut got = Vec::new();
    let mut caught_up = Vec::new();
    for _ in 0..4 {
        let batch = sub.poll(&g, &bob(), 1).unwrap();
        got.extend(batch.messages);
        caught_up.extend(batch.caught_up);
    }
    assert_eq!(bodies(&got), vec!["real"]);
    assert!(got.iter().all(|m| !m.body.contains(&token)));
    assert_eq!(caught_up, vec![2], "caught up once, at the head");
    assert_eq!(sub.last_sent(), Some(2));
}

#[test]
fn a_subscription_that_starts_at_the_canary_still_moves_on() {
    let (g, _) = world();
    let mut sub = Subscription::new("vault", 0, 16);
    let first = sub.poll(&g, &bob(), 1).unwrap();
    // seq 1 is the canary: nothing to show, but the position moved past it
    assert!(first.messages.is_empty());
    assert_eq!(first.caught_up, None);
    assert_eq!(sub.last_sent(), Some(1));
}

#[test]
fn the_canary_is_no_message_to_ask_a_receipt_for() {
    let (g, _) = world();
    let err = g.receipt(&alice(), "vault", 1).unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err}");
}

#[test]
fn the_canary_cannot_be_asked_across_and_asking_trips_nothing() {
    let (g, _) = world();
    g.join(&bob(), "open").unwrap();
    let err = g.request_crossing(&bob(), "vault", 1, "open").unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err}");
    // the target room was not paused by the attempt
    g.post(&bob(), "open", "still open").unwrap();
    assert!(g.store.pending_reviews(10).unwrap().is_empty());
}

#[test]
fn before_any_digest_wait_wake_and_the_outside_view_see_nothing_of_it() {
    let g = open();
    g.admin_set_clearance(&admin(), "bob", "private").unwrap();
    g.join(&bob(), "vault").unwrap();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    assert!(g.wait_poll(&bob(), "vault").unwrap().is_empty());
    assert_eq!(g.wake_summary(&bob(), "vault").unwrap(), (0, 0));
    assert_eq!(g.outside_view(&bob(), "vault").unwrap(), None);
    assert!(g.read(&bob(), "vault", false).unwrap().is_empty());
    assert!(g.read(&bob(), "vault", true).unwrap().is_empty());
}

#[test]
fn a_librarian_peek_wait_and_history_never_show_it() {
    let (g, _) = world();
    g.admin_set_clearance(&admin(), "lib", "private").unwrap();
    g.admin_set_librarian(&admin(), "lib", true).unwrap();
    let lib = Principal::unattributed("lib");
    assert_eq!(bodies(&g.read(&lib, "vault", false).unwrap()), ["real"]);
    assert_eq!(bodies(&g.wait_poll(&lib, "vault").unwrap()), ["real"]);
    assert_eq!(g.wake_summary(&lib, "vault").unwrap(), (1, 2));
    let h = g.history(&lib, "vault", HistoryRange::default()).unwrap();
    assert_eq!(bodies(&h.messages), ["real"]);
}

#[test]
fn the_export_and_tail_window_hold_no_canary_and_the_check_still_finds_the_token() {
    let (g, token) = world();
    // the export (`klatch admin export`) reads through the gateway's tail
    let all = g.tail(&bob(), "vault", (0, 1_000_000)).unwrap();
    assert_eq!(bodies(&all), vec!["real"]);
    let err = g.post(&bob(), "open", &format!("fyi {token}")).unwrap_err();
    assert_eq!(rule_of(err), "canary");
}
