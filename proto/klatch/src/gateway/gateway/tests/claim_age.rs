//! Age reminders for long-held claims (#547), through the gateway and the hook text.

use super::*;
use crate::domain::render_hook;

const TWO_HOURS: i64 = 7_200;

fn long_claim(g: &Gateway<SpyStore>, who: &Principal, path: &str) {
    g.join(who, "r").unwrap();
    g.claim(who, "r", &ClaimSpec::new(path, 3_600, "")).unwrap();
}

#[test]
fn the_owner_is_reminded_once_when_its_claim_passes_two_hours_and_peers_are_not() {
    let (g, t) = timed();
    long_claim(&g, &alice(), "src/a.rs");
    g.join(&bob(), "r").unwrap();
    // Alice stays active, so the claim renews and never lapses.
    for _ in 0..11 {
        t.set(t.get() + 600);
        g.digest(&alice()).unwrap();
    }
    t.set(t.get() + 600);
    let mine = render_hook(&g.digest(&alice()).unwrap().rooms, t.get());
    assert!(
        mine.contains("your claim on src/a.rs has been held for 2h"),
        "{mine}"
    );
    let again = render_hook(&g.digest(&alice()).unwrap().rooms, t.get());
    assert!(!again.contains("has been held"), "told once: {again}");
    let theirs = render_hook(&g.digest(&bob()).unwrap().rooms, t.get());
    assert!(!theirs.contains("has been held"), "{theirs}");
}

#[test]
fn a_young_claim_is_not_reminded() {
    let (g, t) = timed();
    long_claim(&g, &alice(), "src/a.rs");
    t.set(t.get() + TWO_HOURS - 1);
    let out = render_hook(&g.digest(&alice()).unwrap().rooms, t.get());
    assert!(!out.contains("has been held"), "{out}");
}

#[test]
fn a_peer_that_runs_into_a_long_held_claim_is_told_its_age() {
    let (g, t) = timed();
    long_claim(&g, &alice(), "src/a.rs");
    g.join(&bob(), "r").unwrap();
    for _ in 0..13 {
        t.set(t.get() + 600);
        g.digest(&alice()).unwrap();
    }
    let err = g
        .claim(&bob(), "r", &ClaimSpec::new("src/a.rs", 60, ""))
        .unwrap_err();
    assert!(err.to_string().contains("held for 2h"), "{err}");
}

#[test]
fn a_young_claim_does_not_show_an_age_to_a_peer() {
    let (g, _t) = timed();
    long_claim(&g, &alice(), "src/a.rs");
    g.join(&bob(), "r").unwrap();
    let err = g
        .claim(&bob(), "r", &ClaimSpec::new("src/a.rs", 60, ""))
        .unwrap_err();
    assert!(!err.to_string().contains("held for"), "{err}");
}
