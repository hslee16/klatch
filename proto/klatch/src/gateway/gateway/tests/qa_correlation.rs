//! Question and answer correlation (#234): by text, with no reply-to field. An answer names its ask by number
//! (`answer to #<seq>:`), a direct question is a DM, and several answers are all delivered.

use super::*;

fn carol() -> Principal {
    Principal::unattributed("carol")
}

#[test]
fn several_answers_to_one_ask_are_all_delivered_and_the_first_ends_it() {
    let g = open();
    for p in [alice(), bob(), carol()] {
        g.join(&p, "r").unwrap();
    }
    let ask = g.post(&alice(), "r", "seeking: slugify").unwrap().seq;
    assert_eq!(g.open_asks(&bob(), "r").unwrap().len(), 1);
    g.post(&bob(), "r", &format!("answer to #{ask}: use crate a"))
        .unwrap();
    assert!(
        g.open_asks(&bob(), "r").unwrap().is_empty(),
        "the first answer ends the ask"
    );
    g.post(&carol(), "r", &format!("answer to #{ask}: use crate b"))
        .unwrap();
    assert!(g.open_asks(&bob(), "r").unwrap().is_empty());
    let shown: Vec<String> = g
        .read(&alice(), "r", false)
        .unwrap()
        .into_iter()
        .map(|m| m.body)
        .collect();
    assert_eq!(shown.len(), 2, "every answer reaches the asker: {shown:?}");
    assert!(shown[0].ends_with("crate a") && shown[1].ends_with("crate b"));
}

#[test]
fn a_direct_question_is_a_dm_and_never_a_lobby_ask() {
    let g = open();
    g.join(&alice(), "lobby").unwrap();
    g.join(&bob(), "lobby").unwrap();
    let (room, posted) = g
        .dm(&alice(), "bob", "seeking: which crate?", None)
        .unwrap();
    assert!(g.open_asks(&bob(), "lobby").unwrap().is_empty());
    assert_eq!(g.open_asks(&bob(), &room).unwrap().len(), 1);
    g.dm(
        &bob(),
        "alice",
        &format!("answer to #{}: crate a", posted.seq),
        None,
    )
    .unwrap();
    assert!(g.open_asks(&bob(), &room).unwrap().is_empty());
}
