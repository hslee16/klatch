//! #864 (epic #863): the merge queue on the board, derived from `ready:` and the integrator's state words and the
//! claims. The integrator is a member with the role; its words count, a plain member's state words do not.

use super::*;
use crate::domain::{DropReason, QueueState};

fn ivy() -> Principal {
    Principal::unattributed("ivy")
}

fn world() -> (Gateway<SpyStore>, Rc<std::cell::Cell<i64>>) {
    let (g, t) = timed();
    for who in [alice(), bob(), ivy()] {
        g.join(&who, "r").unwrap();
    }
    g.admin_set_integrator(&admin(), "ivy", true).unwrap();
    (g, t)
}

fn queue(g: &Gateway<SpyStore>) -> Vec<(u64, QueueState, Option<usize>)> {
    g.board(&bob(), "r")
        .unwrap()
        .queue
        .iter()
        .map(|e| (e.number, e.state, e.position))
        .collect()
}

#[test]
fn a_ready_word_puts_the_pr_on_the_board_and_the_integrators_words_move_it() {
    let (g, _t) = world();
    g.post(&alice(), "r", "ready: #7").unwrap();
    g.post(&bob(), "r", "ready: #8").unwrap();
    assert_eq!(
        queue(&g),
        [
            (7, QueueState::Waiting, Some(1)),
            (8, QueueState::Waiting, Some(2))
        ]
    );
    g.post(&ivy(), "r", "integrating: #7").unwrap();
    g.post(&ivy(), "r", "checking: #7").unwrap();
    assert_eq!(queue(&g)[0].1, QueueState::Checking);
    g.post(&ivy(), "r", "merged: #7").unwrap();
    g.post(&ivy(), "r", "dropped: #8 conflict").unwrap();
    assert_eq!(
        queue(&g),
        [
            (8, QueueState::Dropped(DropReason::Conflict), None),
            (7, QueueState::Merged, None)
        ]
    );
}

#[test]
fn a_state_word_from_a_plain_member_changes_nothing_and_a_removed_role_stops_counting() {
    let (g, _t) = world();
    g.post(&alice(), "r", "ready: #7").unwrap();
    g.post(&bob(), "r", "integrating: #7").unwrap();
    g.post(&bob(), "r", "dropped: #7 closed").unwrap();
    assert_eq!(queue(&g), [(7, QueueState::Waiting, Some(1))]);
    g.post(&ivy(), "r", "integrating: #7").unwrap();
    assert_eq!(queue(&g)[0].1, QueueState::Integrating);
    // the role is read when the board is built: without it the integrating word is ignored
    g.admin_set_integrator(&admin(), "ivy", false).unwrap();
    assert_eq!(queue(&g)[0].1, QueueState::Waiting);
}

#[test]
fn the_board_text_has_a_queue_section_with_the_merge_claim_holder_and_the_pr_owner() {
    let (g, _t) = world();
    g.post(&alice(), "r", "ready: #7").unwrap();
    g.post(&alice(), "r", "pr: #7 add the thing").unwrap();
    g.claim(&alice(), "r", &ClaimSpec::new("pr:7", 600, "").shared(true))
        .unwrap();
    let text = crate::domain::render_board(&g.board(&bob(), "r").unwrap(), 1_000);
    assert!(
        text.contains("queue (merge:main free):\n  1. #7 waiting (ready by alice"),
        "{text}"
    );
    assert!(text.contains(", owner alice)"), "{text}");
    g.claim(&ivy(), "r", &ClaimSpec::new("merge:main", 600, "merging"))
        .unwrap();
    g.post(&ivy(), "r", "integrating: #7").unwrap();
    let text = crate::domain::render_board(&g.board(&bob(), "r").unwrap(), 1_000);
    assert!(
        text.contains("queue (merge:main held by ivy):\n  1. #7 integrating"),
        "{text}"
    );
}

#[test]
fn a_board_with_no_queue_words_has_no_queue_section() {
    let (g, _t) = world();
    let text = crate::domain::render_board(&g.board(&bob(), "r").unwrap(), 1_000);
    assert!(!text.contains("queue"), "{text}");
}

#[test]
fn queue_words_deep_in_a_busy_room_are_still_read() {
    let g = Gateway::new(SpyStore::new(), FixedClock(1000), AllowAll);
    g.join(&alice(), "r").unwrap();
    g.join(&bob(), "r").unwrap();
    g.store
        .post(Seat::new("r", "alice"), "ready: #7", 10)
        .unwrap();
    for i in 0..5100 {
        g.store
            .post(Seat::new("r", "bob"), &format!("chatter {i}"), 50)
            .unwrap();
    }
    g.store
        .post(Seat::new("r", "alice"), "ready: #9", 900)
        .unwrap();
    let numbers: Vec<u64> = g
        .board(&bob(), "r")
        .unwrap()
        .queue
        .iter()
        .map(|e| e.number)
        .collect();
    assert_eq!(numbers, [7, 9]);
}
