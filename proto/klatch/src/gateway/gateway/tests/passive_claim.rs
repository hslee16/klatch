//! Passive claims through the gateway (#1401): what is claimed, in which room, for how long, and what is counted.

use super::*;
use crate::domain::{PASSIVE_CLAIM_SECS, PassiveOutcome};

fn held(g: &Gateway<SpyStore>, room: &str) -> Vec<(String, String, i64)> {
    g.claims(&bob(), room)
        .unwrap()
        .into_iter()
        .map(|c| (c.owner, c.path, c.expires))
        .collect()
}

#[test]
fn an_edit_takes_a_claim_for_300_seconds_and_a_second_edit_renews_it() {
    let (g, t) = timed();
    g.join(&alice(), "lobby").unwrap();
    g.join(&bob(), "lobby").unwrap();
    assert_eq!(PASSIVE_CLAIM_SECS, 300);
    assert_eq!(
        g.passive_claim(&alice(), "src/a.rs").unwrap(),
        PassiveOutcome::Granted
    );
    assert_eq!(
        held(&g, "lobby"),
        vec![("alice".into(), "src/a.rs".into(), 1_300)]
    );
    t.set(1_200);
    assert_eq!(
        g.passive_claim(&alice(), "src/a.rs").unwrap(),
        PassiveOutcome::Renewed
    );
    assert_eq!(
        held(&g, "lobby"),
        vec![("alice".into(), "src/a.rs".into(), 1_500)]
    );
    // the lease lapses when nothing renews it
    t.set(1_600);
    assert!(held(&g, "lobby").is_empty());
    assert_eq!(
        g.passive_claim(&alice(), "src/a.rs").unwrap(),
        PassiveOutcome::Granted
    );
}

#[test]
fn another_members_claim_refuses_without_a_wait_and_says_who_and_since_when() {
    let (g, t) = timed();
    g.join(&alice(), "lobby").unwrap();
    g.join(&bob(), "lobby").unwrap();
    g.passive_claim(&alice(), "src").unwrap();
    t.set(1_100);
    let PassiveOutcome::Refused(r) = g.passive_claim(&bob(), "src/a.rs").unwrap() else {
        panic!("not refused");
    };
    assert_eq!(
        (r.room.as_str(), r.owner.as_str(), r.path.as_str()),
        ("lobby", "alice", "src")
    );
    assert_eq!(r.held_secs, 100);
    assert_eq!(r.secs_left, 200);
    // nothing was taken, and no wait was recorded: alice claiming what bob holds is no deadlock
    assert_eq!(held(&g, "lobby").len(), 1);
    g.claim(&bob(), "lobby", &ClaimSpec::new("lib/b.rs", 100, ""))
        .unwrap();
    g.claim(&alice(), "lobby", &ClaimSpec::new("lib/b.rs", 100, ""))
        .unwrap_err();
    assert!(
        g.store.wait_cycles("lobby", 1_100).unwrap().is_empty(),
        "a passive refusal is not a wait"
    );
    let c = g.store.passive_counts_for_test();
    assert_eq!((c.granted, c.renewed, c.refused), (1, 0, 1));
}

#[test]
fn an_explicit_claim_that_covers_the_path_is_not_claimed_twice_and_is_never_released() {
    let g = open();
    g.join(&alice(), "lobby").unwrap();
    g.claim(&alice(), "lobby", &ClaimSpec::new("src", 100, "mine"))
        .unwrap();
    assert_eq!(
        g.passive_claim(&alice(), "src/a.rs").unwrap(),
        PassiveOutcome::Covered
    );
    assert_eq!(
        g.passive_claim(&alice(), "src").unwrap(),
        PassiveOutcome::Covered
    );
    g.passive_claim(&alice(), "lib/z.rs").unwrap();
    assert_eq!(g.release_passive_claims(&alice()).unwrap(), 1);
    let left: Vec<String> = held(&g, "lobby").into_iter().map(|c| c.1).collect();
    assert_eq!(left, vec!["src".to_string()]);
}

#[test]
fn release_frees_only_passive_claims_and_counts_them() {
    let g = open();
    g.join(&alice(), "lobby").unwrap();
    g.join(&bob(), "lobby").unwrap();
    g.claim(&alice(), "lobby", &ClaimSpec::new("explicit.rs", 100, ""))
        .unwrap();
    g.passive_claim(&alice(), "a.rs").unwrap();
    g.passive_claim(&alice(), "b.rs").unwrap();
    g.passive_claim(&bob(), "c.rs").unwrap();
    assert_eq!(g.release_passive_claims(&alice()).unwrap(), 2);
    let mut left: Vec<String> = held(&g, "lobby").into_iter().map(|c| c.1).collect();
    left.sort();
    assert_eq!(left, vec!["c.rs".to_string(), "explicit.rs".to_string()]);
    assert_eq!(g.release_passive_claims(&alice()).unwrap(), 0);
    let c = g.store.passive_counts_for_test();
    assert_eq!((c.granted, c.released, c.refused), (3, 2, 0));
    // an explicit release is recorded as an explicit one, not as a passive release
    let h = g.store.claim_history_for_test();
    assert!(
        h.granted.iter().all(|(_, n)| *n == 1),
        "the passive grants are not explicit grants: {h:?}"
    );
}

#[test]
fn a_lapsed_passive_claim_is_counted_as_lapsed_not_as_an_explicit_expiry() {
    let (g, t) = timed();
    g.join(&alice(), "lobby").unwrap();
    g.passive_claim(&alice(), "a.rs").unwrap();
    t.set(2_000);
    assert_eq!(g.release_passive_claims(&alice()).unwrap(), 0);
    let c = g.store.passive_counts_for_test();
    assert_eq!((c.granted, c.released, c.expired), (1, 0, 1));
    assert!(g.store.claim_history_for_test().expired.is_empty());
}

#[test]
fn never_claimed_and_unusable_paths_are_skipped() {
    let g = open();
    g.join(&alice(), "lobby").unwrap();
    for p in [
        "DESIGN.md",
        "CHANGELOG.md",
        "RESEARCH-NOTES.md",
        "/etc/passwd",
        "../x",
        "a\nb",
        "ticket:5",
        "ask:2",
        "",
    ] {
        assert_eq!(
            g.passive_claim(&alice(), p).unwrap(),
            PassiveOutcome::Skipped,
            "{p:?}"
        );
    }
    assert!(held(&g, "lobby").is_empty());
}

#[test]
fn the_lobby_is_the_room_and_ask_rooms_and_direct_messages_are_never_used() {
    let g = open();
    g.join(&alice(), "side").unwrap();
    g.join(&alice(), "ask-7").unwrap();
    g.join(&alice(), "lobby").unwrap();
    g.passive_claim(&alice(), "a.rs").unwrap();
    assert_eq!(held(&g, "lobby").len(), 1);
    assert!(held(&g, "side").is_empty() && held(&g, "ask-7").is_empty());
    // with no lobby the first ordinary room by name is used, and an ask room alone gets nothing
    let g = open();
    g.join(&alice(), "zed").unwrap();
    g.join(&alice(), "alpha").unwrap();
    g.join(&alice(), "ask-1").unwrap();
    g.passive_claim(&alice(), "a.rs").unwrap();
    assert_eq!(held(&g, "alpha").len(), 1);
    assert!(held(&g, "zed").is_empty());
    let g = open();
    g.join(&alice(), "ask-1").unwrap();
    assert_eq!(
        g.passive_claim(&alice(), "a.rs").unwrap(),
        PassiveOutcome::Skipped
    );
    // a member in no room claims nothing
    assert_eq!(
        g.passive_claim(&bob(), "a.rs").unwrap(),
        PassiveOutcome::Skipped
    );
}

#[test]
fn a_room_with_a_sealed_label_is_never_claimed_in() {
    let g = open();
    g.admin_set_room_label(&admin(), "lobby", "private")
        .unwrap();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.join(&alice(), "lobby").unwrap();
    // before the label is sealed a passive claim is allowed; once sealed, none is taken
    assert_eq!(
        g.passive_claim(&alice(), "a.rs").unwrap(),
        PassiveOutcome::Granted
    );
    g.release_passive_claims(&alice()).unwrap();
    g.admin_seal_label_keep_plain(&admin(), "private").unwrap();
    assert_eq!(
        g.passive_claim(&alice(), "b.rs").unwrap(),
        PassiveOutcome::Skipped
    );
    assert!(g.claims(&alice(), "lobby").unwrap().is_empty());
}

#[test]
fn a_room_above_the_members_clearance_is_not_claimed_in() {
    let g = open();
    g.admin_set_room_label(&admin(), "lobby", "private")
        .unwrap();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.join(&alice(), "lobby").unwrap();
    g.admin_set_clearance(&admin(), "alice", "public").unwrap();
    assert_eq!(
        g.passive_claim(&alice(), "a.rs").unwrap(),
        PassiveOutcome::Skipped
    );
}

#[test]
fn a_passive_claim_is_not_announced_in_another_members_digest_but_an_explicit_one_is() {
    let g = open();
    g.join(&alice(), "lobby").unwrap();
    g.join(&bob(), "lobby").unwrap();
    g.passive_claim(&alice(), "src/a.rs").unwrap();
    g.claim(
        &alice(),
        "lobby",
        &ClaimSpec::new("src/explicit.rs", 100, ""),
    )
    .unwrap();
    let d = g.digest(&bob()).unwrap();
    let new: Vec<String> = d
        .rooms
        .iter()
        .flat_map(|r| r.claims.iter().map(|c| c.path.clone()))
        .collect();
    assert_eq!(new, vec!["src/explicit.rs".to_string()], "{new:?}");
}

#[test]
fn a_store_failure_is_returned_and_counted_never_a_panic() {
    let g = open();
    g.join(&alice(), "lobby").unwrap();
    g.store.fail.set(true);
    assert!(g.passive_claim(&alice(), "a.rs").is_err());
    g.store.fail.set(false);
    // a failure is a number in the stats, whether the store's or not
    assert_eq!(
        g.passive_claim(&alice(), "a.rs").unwrap(),
        PassiveOutcome::Granted
    );
    let nameless = Principal::unattributed("");
    assert!(g.passive_claim(&nameless, "a.rs").is_err());
    assert_eq!(g.store.passive_counts_for_test().errors, 2);
}

#[test]
fn the_counts_hold_no_name_and_follow_the_room_filter() {
    let g = open();
    g.join(&alice(), "lobby").unwrap();
    g.passive_claim(&alice(), "SECRET-PATH/x.rs").unwrap();
    g.passive_claim(&alice(), "SECRET-PATH/x.rs").unwrap();
    let opts = crate::domain::StatsOptions {
        top: 5,
        ..Default::default()
    };
    let stats = g.store.stats_for_test(&opts);
    let text = stats.to_text();
    assert!(
        text.contains(
            "passive claims: 1 granted, 1 renewed, 0 refused, 0 released, 0 lapsed, 0 failed"
        ),
        "{text}"
    );
    assert!(!text.contains("SECRET-PATH") && !stats.to_json().to_string().contains("SECRET-PATH"));
    assert_eq!(stats.to_json()["passive_claims"]["renewed"], 1);
    let other = crate::domain::StatsOptions {
        room: Some("elsewhere".into()),
        ..opts
    };
    assert_eq!(g.store.stats_for_test(&other).passive_claims.granted, 0);
}
