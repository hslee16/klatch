//! `history` (#518): the last messages of a room, from everyone, without touching any cursor.

use super::*;
use crate::gateway::HistoryRange;

fn room_of(n: usize) -> Gateway<SpyStore> {
    let g = open();
    g.join(&alice(), "r").unwrap();
    // five senders in turn, so the burst limit of one member does not stop a long room
    for i in 1..=n {
        let sender = Principal::unattributed(&format!("s{}", i % 5));
        g.post(&sender, "r", &format!("m{i}")).unwrap();
    }
    g
}

fn seqs(h: &crate::gateway::History) -> Vec<i64> {
    h.messages.iter().map(|m| m.seq).collect()
}

fn range(since: Option<i64>, last: Option<usize>) -> HistoryRange {
    HistoryRange { since, last }
}

#[test]
fn with_no_range_it_is_the_last_twenty() {
    let g = room_of(30);
    let h = g.history(&alice(), "r", range(None, None)).unwrap();
    assert_eq!(seqs(&h), (11..=30).collect::<Vec<_>>());
    assert_eq!(h.more, 0);
}

#[test]
fn last_takes_the_newest_n_oldest_first() {
    let g = room_of(30);
    let h = g.history(&alice(), "r", range(None, Some(3))).unwrap();
    assert_eq!(seqs(&h), vec![28, 29, 30]);
}

#[test]
fn since_pages_forward_and_counts_what_is_left() {
    let g = room_of(30);
    let h = g.history(&alice(), "r", range(Some(5), None)).unwrap();
    assert_eq!(seqs(&h), (6..=25).collect::<Vec<_>>());
    assert_eq!(h.more, 5, "five messages follow the last one shown");
    let next = g.history(&alice(), "r", range(Some(25), None)).unwrap();
    assert_eq!(seqs(&next), (26..=30).collect::<Vec<_>>());
    assert_eq!(next.more, 0);
}

#[test]
fn since_and_last_together_take_the_newest_n_after_since() {
    let g = room_of(30);
    let h = g.history(&alice(), "r", range(Some(5), Some(3))).unwrap();
    assert_eq!(seqs(&h), vec![28, 29, 30]);
    let all_after = g.history(&alice(), "r", range(Some(28), Some(10))).unwrap();
    assert_eq!(seqs(&all_after), vec![29, 30], "fewer than last exist");
}

#[test]
fn a_huge_last_is_cut_to_the_most_one_call_returns() {
    let g = room_of(250);
    let h = g
        .history(&alice(), "r", range(None, Some(100_000)))
        .unwrap();
    assert_eq!(
        h.messages.len(),
        crate::domain::budget::HISTORY_MAX_MESSAGES
    );
    assert_eq!(h.messages.last().unwrap().seq, 250);
}

#[test]
fn it_shows_everyones_messages_and_moves_no_cursor() {
    let g = room_of(3);
    g.post(&alice(), "r", "mine").unwrap();
    let h = g.history(&alice(), "r", range(None, None)).unwrap();
    assert_eq!(h.messages.len(), 4, "the caller's own message is included");
    let unread = g.read(&alice(), "r", false).unwrap();
    assert_eq!(unread.len(), 3, "history did not read anything for alice");
}

#[test]
fn a_bad_range_is_invalid_before_the_store_is_asked() {
    let g = room_of(1);
    for r in [range(Some(-1), None), range(None, Some(0))] {
        assert!(matches!(
            g.history(&alice(), "r", r),
            Err(Error::Invalid(_))
        ));
    }
    assert!(matches!(
        g.history(&alice(), "bad room!", range(None, None)),
        Err(Error::Invalid(_))
    ));
}
