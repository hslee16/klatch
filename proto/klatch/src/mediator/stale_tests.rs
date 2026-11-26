//! A stale ask's proposal must be the same across mediator restarts until the ask moves to a later stage of its schedule (#1570).

use super::tests::{Gw, lobby, me, mediator, proposals_seen};
use super::*;
use crate::gateway::ClaimSpec;
use crate::gateway::areas::*;
use std::cell::Cell;
use std::rc::Rc;

/// One ask by ann, nobody takes it. The first ask is stale once `STALE_AFTER_SECS` passed.
fn stale_lobby() -> (Gw, Rc<Cell<i64>>) {
    let (g, t) = lobby(&["mediator", "ann", "bo"]);
    me(&g, "ann").post("lobby", "seeking: old").unwrap();
    (g, t)
}

fn pass_at(g: &Gw, t: &Rc<Cell<i64>>, m: &mut Mediator, secs_later: i64) -> Vec<Event> {
    t.set(100 + secs_later);
    m.pass(&me(g, "mediator"), t.get()).unwrap()
}

fn stale_posts(g: &Gw) -> Vec<String> {
    proposals_seen(g, "bo")
        .into_iter()
        .filter(|b| b.contains("no taker"))
        .collect()
}

#[test]
fn a_restarted_mediator_does_not_repeat_a_stale_ask_proposal_a_minute_later() {
    let (g, t) = stale_lobby();
    let ev = pass_at(&g, &t, &mut mediator(), 100);
    assert!(matches!(ev.as_slice(), [Event::Proposed { .. }]), "{ev:?}");
    // a new process over the same store, the same ask a minute later
    let ev = pass_at(&g, &t, &mut mediator(), 160);
    assert!(ev.is_empty(), "{ev:?}");
    assert_eq!(stale_posts(&g).len(), 1);
}

#[test]
fn the_stale_proposal_holds_no_number_that_changes_within_a_stage() {
    let (g, t) = stale_lobby();
    pass_at(&g, &t, &mut mediator(), 100);
    pass_at(&g, &t, &mut mediator(), 400);
    let posts = stale_posts(&g);
    assert_eq!(posts.len(), 1, "{posts:?}");
    for age in ["100s", "400s", "160s"] {
        assert!(!posts[0].contains(age), "{}", posts[0]);
    }
}

#[test]
fn crossing_into_a_later_stage_gives_exactly_one_new_proposal_even_across_restarts() {
    let (g, t) = stale_lobby();
    pass_at(&g, &t, &mut mediator(), 100);
    // a stage later: one new proposal from a fresh run, none from the next fresh run or the same run
    let mut m = mediator();
    let ev = pass_at(&g, &t, &mut m, 1500);
    assert!(matches!(ev.as_slice(), [Event::Proposed { .. }]), "{ev:?}");
    assert!(pass_at(&g, &t, &mut m, 1560).is_empty());
    assert!(pass_at(&g, &t, &mut mediator(), 1600).is_empty());
    assert_eq!(stale_posts(&g).len(), 2);
}

#[test]
fn a_running_mediator_proposes_once_per_stage_and_not_again_inside_it() {
    let (g, t) = stale_lobby();
    let mut m = mediator();
    assert_eq!(pass_at(&g, &t, &mut m, 100).len(), 1);
    assert!(pass_at(&g, &t, &mut m, 300).is_empty());
    assert_eq!(pass_at(&g, &t, &mut m, 1500).len(), 1);
}

#[test]
fn an_answered_ask_gets_no_proposal() {
    let (g, t) = stale_lobby();
    me(&g, "bo").post("lobby", "here is the answer").unwrap();
    me(&g, "bo")
        .claim("lobby", &ClaimSpec::new("ask:1", 900, ""))
        .unwrap();
    assert!(pass_at(&g, &t, &mut mediator(), 100).is_empty());
    assert!(stale_posts(&g).is_empty());
}

#[test]
fn two_stale_asks_do_not_interfere_across_restarts() {
    let (g, t) = stale_lobby();
    // bo's ask is 300s younger, so the two asks cross their stages at different times
    t.set(400);
    me(&g, "bo").post("lobby", "seeking: other").unwrap();
    let ev = pass_at(&g, &t, &mut mediator(), 700);
    assert_eq!(ev.len(), 2, "{ev:?}");
    assert!(pass_at(&g, &t, &mut mediator(), 800).is_empty());
    // only bo's ask is a stage older here: one new proposal, for it alone
    let ev = pass_at(&g, &t, &mut mediator(), 950);
    assert!(
        matches!(ev.as_slice(), [Event::Proposed { key }] if key == "stale_ask:2"),
        "{ev:?}"
    );
    assert!(pass_at(&g, &t, &mut mediator(), 1000).is_empty());
    assert_eq!(stale_posts(&g).len(), 3);
}

#[test]
fn an_hour_long_stale_ask_yields_only_the_stages_reached_even_with_a_restart_at_every_pass() {
    let (g, t) = stale_lobby();
    let mut posted = 0;
    for age in (100..3600).step_by(250) {
        posted += pass_at(&g, &t, &mut mediator(), age).len();
    }
    // stage 0 from 60s, stage 1 from 10m, stage 2 at exactly 1h
    assert_eq!(posted, 2);
    posted += pass_at(&g, &t, &mut mediator(), 3600).len();
    assert_eq!(posted, 3);
    assert_eq!(stale_posts(&g).len(), 3);
}

#[test]
fn a_stale_ask_gets_at_most_five_lines_ever_and_none_after_the_last_stage() {
    let (g, t) = stale_lobby();
    let day = 86_400;
    for age in [100, 700, 4000, 22_000, day + 100] {
        let ev = pass_at(&g, &t, &mut mediator(), age);
        assert_eq!(ev.len(), 1, "age {age}: {ev:?}");
    }
    for age in [day + 200, 2 * day, 7 * day, 30 * day, 31 * day] {
        assert!(
            pass_at(&g, &t, &mut mediator(), age).is_empty(),
            "age {age}"
        );
    }
    // one run that sees the ask first at 30 days posts one line and then none
    let (g, t) = stale_lobby();
    let mut m = mediator();
    assert_eq!(pass_at(&g, &t, &mut m, 30 * day).len(), 1);
    assert!(pass_at(&g, &t, &mut m, 31 * day).is_empty());
    assert_eq!(stale_posts(&g).len(), 1);
}
