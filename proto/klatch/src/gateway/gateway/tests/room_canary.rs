//! #347, D4: labelling a room to private or above by the human plants a canary there by itself. It reads once, in
//! the member's first digest, then is retired from read, history, tail and the digest; a copy of it in another room
//! trips the wire; `admin_retire_canary` ends it.

use super::*;

fn ada() -> Principal {
    Principal::unattributed("ada")
}

/// The planted tokens of `room`.
fn tokens_of(g: &Gateway<SpyStore>, room: &str) -> Vec<String> {
    g.store
        .policy_list("canary:")
        .unwrap()
        .into_iter()
        .filter(|(_, home)| home == room)
        .map(|(key, _)| key.trim_start_matches("canary:").to_string())
        .collect()
}

/// A room `vault` that alice has joined, labelled private by the human, with alice cleared for it.
fn vault() -> (Gateway<SpyStore>, String) {
    let g = open();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.join(&alice(), "vault").unwrap();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    let token = tokens_of(&g, "vault").pop().expect("a canary was planted");
    (g, token)
}

#[test]
fn labelling_a_room_private_plants_a_canary_at_that_label_with_no_other_step() {
    let (g, token) = vault();
    assert!(token.starts_with("kc-canary-"), "{token}");
    let planted = g.store.tail("vault", 0, 10).unwrap();
    assert_eq!(planted.len(), 1);
    assert_eq!(
        (planted[0].sender.as_str(), planted[0].label.as_str()),
        ("canary", "private")
    );
    assert!(planted[0].body.contains(&token));
}

#[test]
fn a_label_below_private_plants_nothing_and_a_higher_label_plants_once() {
    let g = open();
    g.admin_set_room_label(&admin(), "town", "internal")
        .unwrap();
    assert!(tokens_of(&g, "town").is_empty());
    g.admin_set_room_label(&admin(), "town", "private").unwrap();
    g.admin_set_room_label(&admin(), "town", "secret").unwrap();
    g.admin_set_room_label(&admin(), "town", "private").unwrap();
    assert_eq!(tokens_of(&g, "town").len(), 1);
}

#[test]
fn a_canary_token_is_random_never_a_function_of_room_time_and_order() {
    let a = open();
    let b = open();
    for g in [&a, &b] {
        g.admin_set_room_label(&admin(), "vault", "private")
            .unwrap();
        g.admin_set_room_label(&admin(), "other", "private")
            .unwrap();
    }
    let all = [
        tokens_of(&a, "vault"),
        tokens_of(&a, "other"),
        tokens_of(&b, "vault"),
        tokens_of(&b, "other"),
    ]
    .concat();
    let unique: std::collections::HashSet<_> = all.iter().collect();
    assert_eq!(all.len(), 4);
    assert_eq!(unique.len(), 4, "a token repeated");
}

#[test]
fn an_admin_agent_labelling_a_room_does_not_plant() {
    let g = open();
    g.admin_set_admin_agent(&admin(), "ada", true).unwrap();
    g.admin_set_room_label(&ada(), "town", "private").unwrap();
    assert!(tokens_of(&g, "town").is_empty());
    // the human relabelling later plants it
    g.admin_set_room_label(&admin(), "town", "secret").unwrap();
    assert_eq!(tokens_of(&g, "town").len(), 1);
}

#[test]
fn a_room_labelled_before_the_human_relabels_it_has_no_canary() {
    let g = open();
    g.store.policy_set("label:old", Some("private"), 1).unwrap();
    assert!(tokens_of(&g, "old").is_empty());
}

#[test]
fn the_first_digest_shows_it_once_and_then_it_is_gone_from_every_view() {
    let (g, token) = vault();
    let first = g.digest(&alice()).unwrap();
    assert!(
        first.rooms[0]
            .messages
            .iter()
            .any(|m| m.body.contains(&token))
    );
    let again = g.digest(&alice()).unwrap();
    assert!(again.rooms[0].messages.is_empty());
    assert!(g.wait_poll(&alice(), "vault").unwrap().is_empty());
    assert_eq!(g.wakeable(&alice(), "vault").unwrap(), 0);
}

#[test]
fn read_tail_and_history_never_show_it_even_before_the_digest() {
    let (g, _) = vault();
    assert!(g.tail(&alice(), "vault", (0, 10)).unwrap().is_empty());
    assert!(
        g.history(&alice(), "vault", HistoryRange::default())
            .unwrap()
            .messages
            .is_empty()
    );
    assert!(g.read(&alice(), "vault", false).unwrap().is_empty());
}

#[test]
fn a_member_below_the_label_never_sees_it() {
    let (g, token) = vault();
    g.join(&bob(), "elsewhere").unwrap();
    assert!(
        g.tail(&bob(), "vault", (0, 10))
            .unwrap()
            .iter()
            .all(|m| !m.body.contains(&token))
    );
    assert!(g.read(&bob(), "vault", false).is_err());
    let d = g.digest(&bob()).unwrap();
    assert!(
        d.rooms
            .iter()
            .all(|r| r.messages.iter().all(|m| !m.body.contains(&token)))
    );
}

#[test]
fn a_copy_in_another_room_trips_the_wire() {
    let (g, token) = vault();
    let err = g
        .post(&alice(), "town", &format!("fyi {token}"))
        .unwrap_err();
    assert_eq!(rule_of(err), "canary");
}

#[test]
fn retiring_a_canary_ends_the_tripwire_and_only_the_human_may_retire() {
    let (g, token) = vault();
    g.admin_set_admin_agent(&admin(), "ada", true).unwrap();
    assert_eq!(
        rule_of(g.admin_retire_canary(&ada(), "vault").unwrap_err()),
        "admin-scope"
    );
    assert_eq!(tokens_of(&g, "vault").len(), 1);
    g.admin_retire_canary(&admin(), "vault").unwrap();
    assert!(tokens_of(&g, "vault").is_empty());
    g.post(&alice(), "town", &format!("fyi {token}")).unwrap();
    assert!(is_invalid(g.admin_retire_canary(&admin(), "vault")));
}

#[test]
fn a_retired_room_gets_a_fresh_canary_when_the_human_relabels_it() {
    let (g, old) = vault();
    g.admin_retire_canary(&admin(), "vault").unwrap();
    g.admin_set_room_label(&admin(), "vault", "secret").unwrap();
    let fresh = tokens_of(&g, "vault");
    assert_eq!(fresh.len(), 1);
    assert_ne!(fresh[0], old);
}
