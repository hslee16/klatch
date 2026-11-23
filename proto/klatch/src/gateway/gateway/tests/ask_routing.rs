//! Ask routing (#1407): when an ask is posted the gateway addresses it to the members that could take it (`could_answer`
//! and the structured fields of the ask), escalates it to the coordinators when nobody takes it within the lease, and
//! records the outcomes as counts. It only routes: nothing is posted, claimed, answered or closed for anyone.

use super::*;
use crate::domain::{AskRef, Importance, RouteKind};

const ASK: &str = "seeking: a retry limit";

fn who(name: &str) -> Principal {
    Principal::unattributed(name)
}

/// A room "r" with alice (the asker), and the named members joined.
fn room_with(members: &[&str]) -> Gateway<SpyStore> {
    let g = open();
    g.join(&alice(), "r").unwrap();
    for m in members {
        g.join(&who(m), "r").unwrap();
    }
    g
}

fn routed_to(g: &Gateway<SpyStore>, seq: i64) -> Vec<(String, RouteKind)> {
    g.ask_routes(&alice(), "r")
        .unwrap()
        .into_iter()
        .filter(|r| r.seq == seq)
        .map(|r| (r.member, r.kind))
        .collect()
}

fn names(g: &Gateway<SpyStore>, seq: i64) -> Vec<String> {
    routed_to(g, seq).into_iter().map(|(m, _)| m).collect()
}

fn outcomes(g: &Gateway<SpyStore>, seq: i64) -> Vec<String> {
    g.store.route_outcomes("r", seq).unwrap()
}

fn off(g: &Gateway<SpyStore>) {
    g.store
        .policy_set("room:r:ask-routing", Some("off"), 1)
        .unwrap();
}

#[test]
fn routing_picks_the_capable_member_by_scope_and_role() {
    let g = room_with(&["bob", "carol", "dave", "erin", "fay"]);
    g.set_scope(&who("bob"), "git:web").unwrap();
    g.set_scope(&who("carol"), "git:db").unwrap();
    g.admin_set_librarian(&admin(), "erin", true).unwrap();
    // dave holds a claim and is busy
    g.claim(&who("dave"), "r", &ClaimSpec::new("src/x.rs", 600, ""))
        .unwrap();
    let seq = g
        .post_scoped(&alice(), ("r", ASK, None), "git:web")
        .unwrap()
        .seq;
    // bob has the scope, fay declared none (scope is advisory), carol works elsewhere, dave is busy, erin only looks
    assert_eq!(names(&g, seq), ["bob", "fay"]);
    assert!(
        routed_to(&g, seq)
            .iter()
            .all(|(_, k)| *k == RouteKind::Capable)
    );
}

#[test]
fn an_incapable_member_is_not_addressed_and_the_asker_never_is() {
    let g = room_with(&["bob", "carol", "dave"]);
    g.admin_set_reviewer(&admin(), "bob", true).unwrap();
    g.admin_remove_member(&admin(), "r", "carol").unwrap();
    let seq = g.post(&alice(), "r", ASK).unwrap().seq;
    assert_eq!(names(&g, seq), ["dave"]);
    assert!(g.routed_asks("r", "bob").unwrap().is_empty());
    assert!(g.routed_asks("r", "alice").unwrap().is_empty());
    assert_eq!(g.routed_asks("r", "dave").unwrap(), [seq]);
}

#[test]
fn the_best_matches_come_first_and_at_most_three_are_addressed() {
    let g = room_with(&["b1", "b2", "b3", "b4", "b5"]);
    g.set_scope(&who("b4"), "git:web").unwrap();
    let refs = [AskRef {
        kind: "repo".into(),
        value: "git:web".into(),
        ..Default::default()
    }];
    let seq = g
        .post_ask(&alice(), ("r", ASK, None), None, &refs)
        .unwrap()
        .seq;
    let got = names(&g, seq);
    assert_eq!(got.len(), 3);
    assert!(got.contains(&"b4".to_string()), "{got:?}");
}

#[test]
fn a_sealed_room_routes_only_to_members_cleared_for_its_label() {
    let g = room_with(&["bob", "carol"]);
    g.admin_set_clearance(&admin(), "alice", "internal")
        .unwrap();
    g.admin_set_clearance(&admin(), "bob", "internal").unwrap();
    g.admin_set_room_label(&admin(), "r", "internal").unwrap();
    let seq = g.post(&alice(), "r", ASK).unwrap().seq;
    assert_eq!(names(&g, seq), ["bob"]);
    assert!(g.routed_asks("r", "carol").unwrap().is_empty());
    // a grant for the room counts, as it does for could_answer, but only for asks posted after it
    g.grant_room_clearance(&alice(), "r", "carol", "internal")
        .unwrap();
    let later = g.post(&alice(), "r", "seeking: another").unwrap().seq;
    assert_eq!(names(&g, later), ["bob", "carol"]);
}

#[test]
fn a_taken_ask_stops_routing_and_is_counted_once() {
    let g = room_with(&["bob", "carol"]);
    let seq = g.post(&alice(), "r", ASK).unwrap().seq;
    assert_eq!(g.routed_asks("r", "carol").unwrap(), [seq]);
    g.claim(
        &who("bob"),
        "r",
        &ClaimSpec::new(&format!("ask:{seq}"), 60, ""),
    )
    .unwrap();
    assert!(g.routed_asks("r", "carol").unwrap().is_empty());
    assert!(g.routed_asks("r", "bob").unwrap().is_empty());
    g.digest(&who("carol")).unwrap();
    g.digest(&who("carol")).unwrap();
    assert_eq!(
        outcomes(&g, seq).iter().filter(|o| *o == "taken").count(),
        1
    );
    assert!(!outcomes(&g, seq).contains(&"escalated".to_string()));
}

#[test]
fn an_answered_ask_is_no_longer_routed() {
    let g = room_with(&["bob", "carol"]);
    let seq = g.post(&alice(), "r", ASK).unwrap().seq;
    g.post(&bob(), "r", &format!("answer to #{seq}: three"))
        .unwrap();
    assert!(g.routed_asks("r", "carol").unwrap().is_empty());
}

fn with_coordinator() -> (Gateway<SpyStore>, Rc<std::cell::Cell<i64>>) {
    let (g, t) = timed();
    for m in ["alice", "bob", "coord"] {
        g.join(&who(m), "r").unwrap();
    }
    g.admin_set_integrator(&admin(), "coord", true).unwrap();
    // the coordinator is busy: it still gets an escalation, but is not asked first
    g.claim(&who("coord"), "r", &ClaimSpec::new("merge:q", 900, ""))
        .unwrap();
    (g, t)
}

#[test]
fn an_ask_nobody_takes_within_the_lease_escalates_once_to_the_coordinator() {
    let (g, t) = with_coordinator();
    let seq = g.post(&alice(), "r", ASK).unwrap().seq;
    assert_eq!(names(&g, seq), ["bob"], "the busy coordinator is not first");
    t.set(t.get() + 89);
    g.digest(&who("bob")).unwrap();
    assert!(!outcomes(&g, seq).contains(&"escalated".to_string()));
    t.set(t.get() + 1);
    g.digest(&who("bob")).unwrap();
    g.digest(&who("coord")).unwrap();
    g.stuck(&alice(), "r").unwrap();
    assert!(
        routed_to(&g, seq).contains(&("coord".to_string(), RouteKind::Coordinator)),
        "{:?}",
        routed_to(&g, seq)
    );
    assert_eq!(g.routed_asks("r", "coord").unwrap(), [seq]);
    let o = outcomes(&g, seq);
    assert_eq!(o.iter().filter(|x| *x == "escalated").count(), 1, "{o:?}");
}

#[test]
fn a_lapsed_ask_escalates_once_and_counts_taken_and_lapsed() {
    let (g, t) = with_coordinator();
    let seq = g.post(&alice(), "r", ASK).unwrap().seq;
    g.claim(
        &who("bob"),
        "r",
        &ClaimSpec::new(&format!("ask:{seq}"), 60, ""),
    )
    .unwrap();
    g.digest(&who("coord")).unwrap();
    assert!(outcomes(&g, seq).contains(&"taken".to_string()));
    t.set(t.get() + 61);
    g.digest(&who("coord")).unwrap();
    g.digest(&who("coord")).unwrap();
    let o = outcomes(&g, seq);
    assert_eq!(o.iter().filter(|x| *x == "lapsed").count(), 1, "{o:?}");
    assert_eq!(o.iter().filter(|x| *x == "escalated").count(), 1, "{o:?}");
    assert_eq!(g.routed_asks("r", "coord").unwrap(), [seq]);
}

#[test]
fn an_escalation_with_no_coordinator_is_recorded_as_unreached() {
    let (g, t) = timed();
    g.join(&alice(), "r").unwrap();
    let seq = g.post(&alice(), "r", ASK).unwrap().seq;
    t.set(t.get() + 100);
    g.digest(&alice()).unwrap();
    assert!(outcomes(&g, seq).contains(&"escalated".to_string()));
    assert!(g.routed_asks("r", "alice").unwrap().is_empty());
}

#[test]
fn a_direct_message_ask_is_not_routed() {
    let g = open();
    let (room, _) = g.dm(&alice(), "bob", "seeking: the port", None).unwrap();
    assert!(g.store.ask_routes(&room).unwrap().is_empty());
    assert!(g.store.route_outcomes(&room, 1).unwrap().is_empty());
}

#[test]
fn routing_posts_nothing_and_stores_only_ids_names_and_numbers() {
    let g = room_with(&["bob", "carol"]);
    let seq = g.post(&alice(), "r", ASK).unwrap().seq;
    // the room holds the ask and nothing else: no routing message, no claim, no answer
    let log = g.tail(&alice(), "r", (0, 20)).unwrap();
    assert_eq!(log.len(), 1, "{log:?}");
    assert!(g.claims(&alice(), "r").unwrap().is_empty());
    let routes = g.store.ask_routes("r").unwrap();
    assert_eq!(routes.len(), 2);
    assert_eq!(outcomes(&g, seq), ["routed"]);
}

#[test]
fn a_room_can_switch_routing_off_and_the_ask_is_then_only_a_message() {
    let g = room_with(&["bob"]);
    off(&g);
    let seq = g.post(&alice(), "r", ASK).unwrap().seq;
    assert!(names(&g, seq).is_empty());
    assert!(outcomes(&g, seq).is_empty());
}

// The digest and the wake.

/// carol narrowed her delivery view to her own scope, and chatter and the ask come from outside it.
fn narrowed() -> Gateway<SpyStore> {
    let g = room_with(&["bob", "carol"]);
    g.set_scope(&who("carol"), "git:db").unwrap();
    g.set_scope(&who("bob"), "git:web").unwrap();
    g.set_delivery_view(&who("carol"), crate::gateway::DeliveryView::Scope)
        .unwrap();
    g.post(&bob(), "r", "chatter one").unwrap();
    g.post(&bob(), "r", "chatter two").unwrap();
    g
}

#[test]
fn the_digest_shows_a_routed_ask_first_and_a_narrowed_view_does_not_hide_it() {
    let g = narrowed();
    let seq = g.post(&alice(), "r", ASK).unwrap().seq;
    let d = g.digest(&who("carol")).unwrap();
    let room = &d.rooms[0];
    assert!(room.messages.iter().any(|m| m.seq == seq), "{room:?}");
    assert!(room.addressed.contains(&seq));
    let class = room.ranks.iter().find(|(s, _)| *s == seq).unwrap().1;
    assert_eq!(class, Importance::Ask);
    // the chatter stays outside her narrowed view
    assert_eq!(room.messages.len(), 1);
}

#[test]
fn without_routing_the_same_narrowed_member_does_not_see_the_ask() {
    let g = narrowed();
    off(&g);
    g.post(&alice(), "r", ASK).unwrap();
    let d = g.digest(&who("carol")).unwrap();
    assert!(d.rooms[0].messages.is_empty());
}

#[test]
fn a_wake_opted_idle_member_in_a_muted_room_is_woken_only_by_a_routed_ask() {
    let g = room_with(&["bob", "carol", "dave"]);
    g.claim(&who("dave"), "r", &ClaimSpec::new("src/x.rs", 600, ""))
        .unwrap();
    for m in ["carol", "dave"] {
        g.set_muted(&who(m), "r", true).unwrap();
    }
    g.post(&bob(), "r", "chatter").unwrap();
    assert_eq!(g.wakeable(&who("carol"), "r").unwrap(), 0);
    let seq = g.post(&alice(), "r", ASK).unwrap().seq;
    assert_eq!(g.wakeable(&who("carol"), "r").unwrap(), 1);
    assert_eq!(g.wake_summary(&who("carol"), "r").unwrap(), (1, seq));
    // dave is busy, was not routed, and stays asleep
    assert_eq!(g.wakeable(&who("dave"), "r").unwrap(), 0);
}

// The measure: scripted steps from the ask to a capable member seeing it, with and without routing. A step is one hook
// turn of the member (a digest), 30 seconds after the last. Each member is measured in a world of its own, so one
// member's steps do not move another's clock. None means not seen within ten steps.

fn steps_to_see(routing: bool, member: &str) -> Option<u32> {
    let (g, t) = timed();
    for m in ["alice", "bob", "carol", "coord"] {
        g.join(&who(m), "r").unwrap();
    }
    g.admin_set_integrator(&admin(), "coord", true).unwrap();
    g.set_scope(&who("carol"), "git:db").unwrap();
    g.set_delivery_view(&who("carol"), crate::gateway::DeliveryView::Scope)
        .unwrap();
    g.set_muted(&who("coord"), "r", true).unwrap();
    // the coordinator is busy merging, so it is not among the first addressed
    g.claim(&who("coord"), "r", &ClaimSpec::new("merge:q", 900, ""))
        .unwrap();
    if !routing {
        off(&g);
    }
    let seq = g.post(&alice(), "r", ASK).unwrap().seq;
    for step in 1..=10 {
        t.set(t.get() + 30);
        let d = g.digest(&who(member)).unwrap();
        if d.rooms.iter().any(|r| {
            r.messages.iter().any(|m| m.seq == seq) || r.backlog.iter().any(|a| a.seq == seq)
        }) {
            return Some(step);
        }
    }
    None
}

#[test]
fn steps_from_the_ask_to_the_capable_member_seeing_it_with_and_without_routing() {
    // bob reads everything: the ask is the first thing he sees either way, so routing changes nothing for him
    assert_eq!(steps_to_see(true, "bob"), Some(1));
    assert_eq!(steps_to_see(false, "bob"), Some(1));
    // carol narrowed her view to her own scope: without routing the ask never reaches her digest, with it she sees it
    // on her first turn
    assert_eq!(steps_to_see(true, "carol"), Some(1));
    assert_eq!(steps_to_see(false, "carol"), None);
    // coord muted the room and is busy merging: it hears of an ask nobody took only when the lease of 90 seconds ran out (the third step)
    assert_eq!(steps_to_see(true, "coord"), Some(3));
    assert_eq!(steps_to_see(false, "coord"), None);
}

#[test]
fn an_escalated_ask_wakes_a_muted_coordinator_once_and_the_digest_lists_it_once() {
    let (g, t) = with_coordinator();
    g.set_muted(&who("coord"), "r", true).unwrap();
    let seq = g.post(&alice(), "r", ASK).unwrap().seq;
    // the coordinator's hook already passed the ask before the lease ran out; the mute held it back
    g.digest(&who("coord")).unwrap();
    assert_eq!(g.wake_summary(&who("coord"), "r").unwrap(), (0, 0));
    t.set(t.get() + 90);
    assert_eq!(g.wake_summary(&who("coord"), "r").unwrap(), (1, seq));
    g.wake_mark(&who("coord"), "r").unwrap();
    assert_eq!(g.wake_summary(&who("coord"), "r").unwrap(), (0, 0));
    // the digest still lists it, once, and not again
    let d = g.digest(&who("coord")).unwrap();
    assert!(d.rooms[0].backlog.iter().any(|a| a.seq == seq));
    let again = g.digest(&who("coord")).unwrap();
    assert!(again.rooms.iter().all(|r| r.backlog.is_empty()));
    assert!(g.routed_asks("r", "coord").unwrap().contains(&seq));
}

#[test]
fn a_coordinator_that_cannot_read_a_sealed_room_is_not_escalated_to() {
    let (g, t) = timed();
    for m in ["alice", "coord"] {
        g.join(&who(m), "r").unwrap();
    }
    g.admin_set_integrator(&admin(), "coord", true).unwrap();
    g.admin_set_clearance(&admin(), "alice", "internal")
        .unwrap();
    g.admin_set_room_label(&admin(), "r", "internal").unwrap();
    let seq = g.post(&alice(), "r", ASK).unwrap().seq;
    assert!(names(&g, seq).is_empty());
    t.set(t.get() + 100);
    g.stuck(&alice(), "r").unwrap();
    assert!(outcomes(&g, seq).contains(&"escalated".to_string()));
    assert!(g.routed_asks("r", "coord").unwrap().is_empty());
    let ev = g.store.route_events("r").unwrap();
    assert!(ev.iter().any(|(s, o, _)| *s == seq && o == "escalated"));
}
