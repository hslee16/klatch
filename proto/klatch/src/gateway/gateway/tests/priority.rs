//! What a capped view shows first (#1405): the class of an unread message comes from structured facts only.

use super::*;
use crate::domain::Importance;

fn classes(g: &Gateway<SpyStore>, who: &Principal, room: &str) -> Vec<(i64, Importance)> {
    let unread = g.read(who, room, false).unwrap();
    g.read_ranked(who, room)
        .unwrap()
        .into_iter()
        .map(|(c, m)| {
            assert!(unread.iter().any(|u| u.seq == m.seq));
            (m.seq, c)
        })
        .collect()
}

fn class_of(cs: &[(i64, Importance)], seq: i64) -> Importance {
    cs.iter().find(|(s, _)| *s == seq).unwrap().1
}

#[test]
fn an_ask_to_me_an_open_ask_i_could_answer_and_an_answer_to_my_ask_are_asks() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.post(&alice(), "r", "seeking: a retry limit").unwrap(); // 1, alice's own ask
    g.post(&bob(), "r", "seeking: @alice the port?").unwrap(); // 2
    g.post(&bob(), "r", "seeking: anyone with a docker host")
        .unwrap(); // 3, open, alice could answer
    g.post(&bob(), "r", "answer to #1: three").unwrap(); // 4
    g.post(&bob(), "r", "an ordinary line").unwrap(); // 5
    let cs = classes(&g, &alice(), "r");
    assert_eq!(class_of(&cs, 2), Importance::Ask);
    assert_eq!(class_of(&cs, 3), Importance::Ask);
    assert_eq!(class_of(&cs, 4), Importance::Ask);
    assert_eq!(class_of(&cs, 5), Importance::Rest);
    // shown in sequence order inside the class, the rest after
    let order: Vec<i64> = g
        .read_ranked(&alice(), "r")
        .unwrap()
        .iter()
        .map(|(_, m)| m.seq)
        .collect();
    assert_eq!(order, vec![2, 3, 4, 5]);
}

#[test]
fn an_open_ask_that_is_claimed_or_in_another_scope_is_not_one_i_could_answer() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.post(&bob(), "r", "seeking: one").unwrap(); // 1
    g.post(&bob(), "r", "seeking: two").unwrap(); // 2
    g.claim(
        &Principal::unattributed("carol"),
        "r",
        &ClaimSpec::new("ask:1", 60, ""),
    )
    .unwrap();
    let cs = classes(&g, &alice(), "r");
    assert_eq!(class_of(&cs, 1), Importance::Rest, "someone is on it");
    assert_eq!(class_of(&cs, 2), Importance::Ask);
}

#[test]
fn an_ask_of_my_own_is_not_an_ask_for_me() {
    let g = open();
    g.join(&bob(), "r").unwrap();
    g.post(&alice(), "r", "seeking: something @bob").unwrap();
    let cs = classes(&g, &alice(), "r");
    assert!(cs.is_empty(), "own messages are not unread");
}

#[test]
fn every_message_of_my_direct_message_room_is_a_dm_and_a_mention_or_broadcast_is_addressed() {
    let g = open();
    let (room, _) = g.dm(&bob(), "alice", "hello", None).unwrap();
    g.dm(&bob(), "alice", "again", None).unwrap();
    let cs = classes(&g, &alice(), &room);
    assert!(cs.iter().all(|(_, c)| *c == Importance::Dm), "{cs:?}");
    g.join(&alice(), "r").unwrap();
    g.post(&bob(), "r", "please @alice look").unwrap(); // 1
    g.post(&bob(), "r", "to everyone @all").unwrap(); // 2
    g.post(&bob(), "r", "mentions @alice.smith only").unwrap(); // 3
    g.post(&bob(), "r", "ALERT URGENT DECISION made, alice")
        .unwrap(); // 4
    let cs = classes(&g, &alice(), "r");
    assert_eq!(class_of(&cs, 1), Importance::Addressed);
    assert_eq!(class_of(&cs, 2), Importance::Addressed);
    assert_eq!(class_of(&cs, 3), Importance::Rest, "another member's name");
    assert_eq!(
        class_of(&cs, 4),
        Importance::Rest,
        "words in the text never raise a message"
    );
}

#[test]
fn a_post_by_a_coordinating_role_is_addressed_and_an_ordinary_member_is_not() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.admin_set_integrator(&admin(), "intg", true).unwrap();
    g.join(&Principal::unattributed("intg"), "r").unwrap();
    g.post(&bob(), "r", "plain").unwrap(); // 1
    g.post(&Principal::unattributed("intg"), "r", "a routing note")
        .unwrap(); // 2
    let cs = classes(&g, &alice(), "r");
    assert_eq!(class_of(&cs, 1), Importance::Rest);
    assert_eq!(class_of(&cs, 2), Importance::Addressed);
}

#[test]
fn ranking_moves_no_cursor_and_marking_shown_moves_only_over_what_was_named() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    for i in 1..=5 {
        g.post(&bob(), "r", &format!("m{i}")).unwrap();
    }
    g.post(&bob(), "r", "seeking: @alice help").unwrap(); // 6
    let first = g.read_ranked(&alice(), "r").unwrap();
    assert_eq!(first[0].1.seq, 6);
    assert_eq!(
        g.read(&alice(), "r", false).unwrap().len(),
        6,
        "a look moves nothing"
    );
    g.mark_shown(&alice(), "r", crate::store::Shown::Read, &[1, 2])
        .unwrap();
    let left: Vec<i64> = g
        .read(&alice(), "r", false)
        .unwrap()
        .iter()
        .map(|m| m.seq)
        .collect();
    assert_eq!(left, vec![3, 4, 5, 6]);
}
