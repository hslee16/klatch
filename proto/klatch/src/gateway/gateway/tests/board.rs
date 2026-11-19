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
    g.store.wait_poll(Seat::new("r", "bob"), 4990).unwrap();
    let b = g.board(&alice(), "r").unwrap();
    assert_eq!(
        b.offers.len(),
        1,
        "a waiting member lost its offer to the clock"
    );
}

/// Fill `room` with enough chatter that its first 5000 messages hold no task or PR word.
fn bury(g: &Gateway<SpyStore>, room: &str) {
    for i in 0..5100 {
        g.store
            .post(Seat::new(room, "bob"), &format!("chatter {i}"), 50)
            .unwrap();
    }
}

#[test]
fn a_task_whose_words_are_deep_in_a_busy_room_is_still_on_the_board() {
    let g = Gateway::new(SpyStore::new(), FixedClock(1000), AllowAll);
    g.join(&alice(), "r").unwrap();
    g.store
        .post(Seat::new("r", "alice"), "working: long job", 10)
        .unwrap();
    bury(&g, "r");
    g.store
        .post(Seat::new("r", "alice"), "checkpoint: step 3", 900)
        .unwrap();
    g.store
        .post(Seat::new("r", "bob"), "working: late job", 950)
        .unwrap();
    let b = g.board(&alice(), "r").unwrap();
    let titles: Vec<_> = b.tasks.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(titles, ["long job", "late job"], "{titles:?}");
    let cp = b.tasks[0].checkpoint.as_ref().expect("checkpoint kept");
    assert_eq!(cp.text, "step 3");
}

#[test]
fn an_open_pr_deep_in_a_busy_room_stays_on_the_board_and_in_the_digest_backlog() {
    let g = Gateway::new(SpyStore::new(), FixedClock(1000), AllowAll);
    g.join(&alice(), "r").unwrap();
    g.join(&bob(), "r").unwrap();
    g.store
        .post(Seat::new("r", "alice"), "pr: #7 old pr", 10)
        .unwrap();
    g.store
        .post(Seat::new("r", "alice"), "pr: #8 over pr", 11)
        .unwrap();
    bury(&g, "r");
    g.store
        .post(Seat::new("r", "alice"), "merged: #8", 900)
        .unwrap();
    g.store
        .post(Seat::new("r", "alice"), "pr: #9 late pr", 950)
        .unwrap();
    let b = g.board(&bob(), "r").unwrap();
    let numbers: Vec<_> = b.prs.iter().map(|p| p.number).collect();
    assert_eq!(numbers, [7, 9]);
    let d = g.digest_with(&bob(), true).unwrap();
    let owed: Vec<_> = d.rooms[0].unowned_prs.iter().map(|p| p.number).collect();
    assert_eq!(owed, [7, 9]);
}

fn offer_of(b: &crate::domain::Board, who: &str) -> bool {
    b.offers.iter().any(|o| o.sender == who)
}

#[test]
fn a_member_that_posts_again_after_done_is_back_on_the_board_with_its_offers() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.join(&bob(), "r").unwrap();
    g.set_status(&bob(), "r", "done").unwrap();
    g.post(&bob(), "r", "offering: helpers").unwrap();
    let b = g.board(&alice(), "r").unwrap();
    let bob_row = b.members.iter().find(|m| m.name == "bob").unwrap();
    assert_eq!(bob_row.status, None, "done stayed after the member posted");
    assert!(offer_of(&b, "bob"), "the returned member's offer is hidden");
}

#[test]
fn a_member_that_claims_again_after_done_is_no_longer_done() {
    let g = open();
    g.join(&bob(), "r").unwrap();
    g.set_status(&bob(), "r", "done leaving").unwrap();
    g.claim(&bob(), "r", &ClaimSpec::new("src/a.rs", 100, ""))
        .unwrap();
    assert_eq!(g.who(&bob(), "r").unwrap()[0].status, None);
}

#[test]
fn a_post_keeps_a_status_that_is_not_done_and_a_refused_post_keeps_done() {
    let g = open();
    g.join(&bob(), "r").unwrap();
    g.set_status(&bob(), "r", "busy").unwrap();
    g.post(&bob(), "r", "hello").unwrap();
    assert_eq!(
        g.who(&bob(), "r").unwrap()[0].status.as_deref(),
        Some("busy")
    );
    g.set_status(&bob(), "r", "done").unwrap();
    assert!(g.post(&bob(), "r", "   ").is_err());
    assert_eq!(
        g.who(&bob(), "r").unwrap()[0].status.as_deref(),
        Some("done")
    );
}

#[test]
fn available_keeps_the_offers_on_the_board_and_a_set_status_carries_its_time() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.post(&bob(), "r", "offering: helpers").unwrap();
    g.set_status(&bob(), "r", "available").unwrap();
    let b = g.board(&alice(), "r").unwrap();
    assert!(offer_of(&b, "bob"));
    let bob_row = b.members.iter().find(|m| m.name == "bob").unwrap();
    assert_eq!(
        (bob_row.status.as_deref(), bob_row.status_ts),
        (Some("available"), 100)
    );
}
