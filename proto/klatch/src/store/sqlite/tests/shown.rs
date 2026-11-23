//! The messages a capped view showed out of order (#1405): the cursor moves only over a contiguous shown run, the seqs shown
//! beyond it are remembered until the cursor passes them, and nothing that was not named is marked.

use super::*;
use crate::store::Shown;

fn room_of(n: i64) -> SqliteStore {
    let s = store();
    s.join("r", "alice", 1).unwrap();
    for i in 1..=n {
        s.post(Seat::new("r", "bob"), &format!("m{i}"), i).unwrap();
    }
    s
}

fn unread(s: &SqliteStore) -> Vec<i64> {
    s.read(Seat::new("r", "alice"), false, 99)
        .unwrap()
        .iter()
        .map(|m| m.seq)
        .collect()
}

fn mark(s: &SqliteStore, channel: Shown, seqs: &[i64]) {
    let seqs: Vec<Seq> = seqs.iter().map(|s| Seq(*s)).collect();
    s.mark_shown(Seat::new("r", "alice"), channel, &seqs, 50)
        .unwrap();
}

fn cursor(s: &SqliteStore) -> i64 {
    s.conn
        .query_row(
            "SELECT last_seq FROM members WHERE room='r' AND name='alice'",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

fn remembered(s: &SqliteStore) -> i64 {
    s.conn
        .query_row("SELECT COUNT(*) FROM read_shown", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn a_shown_prefix_moves_the_cursor_to_its_end_and_remembers_nothing() {
    let s = room_of(10);
    mark(&s, Shown::Read, &[1, 2, 3]);
    assert_eq!(cursor(&s), 3);
    assert_eq!(unread(&s), (4..=10).collect::<Vec<_>>());
    assert_eq!(remembered(&s), 0);
}

#[test]
fn messages_shown_past_a_gap_are_remembered_and_not_returned_again() {
    let s = room_of(10);
    mark(&s, Shown::Read, &[9, 10, 1, 2]);
    assert_eq!(cursor(&s), 2, "only the contiguous run moves the cursor");
    assert_eq!(unread(&s), (3..=8).collect::<Vec<_>>(), "9 and 10 are read");
    assert_eq!(remembered(&s), 2);
}

#[test]
fn the_cursor_jumps_over_remembered_seqs_when_the_gap_is_filled_and_they_are_forgotten() {
    let s = room_of(10);
    mark(&s, Shown::Read, &[9, 10, 1, 2]);
    mark(&s, Shown::Read, &[3, 4, 5, 6, 7]);
    assert_eq!(cursor(&s), 7);
    mark(&s, Shown::Read, &[8]);
    assert_eq!(cursor(&s), 10, "8 closes the gap up to 10");
    assert_eq!(remembered(&s), 0);
    assert!(unread(&s).is_empty());
}

#[test]
fn a_seq_that_is_not_an_unread_message_of_another_member_is_ignored() {
    let s = room_of(3);
    s.post(Seat::new("r", "alice"), "mine", 20).unwrap(); // seq 4, alice's own
    mark(&s, Shown::Read, &[4, 99, 0, -5]);
    assert_eq!(remembered(&s), 0);
    assert_eq!(cursor(&s), 0);
    assert_eq!(unread(&s), vec![1, 2, 3]);
}

#[test]
fn own_messages_between_shown_ones_do_not_stop_the_cursor() {
    let s = room_of(2);
    s.post(Seat::new("r", "alice"), "mine", 20).unwrap(); // seq 3
    s.post(Seat::new("r", "bob"), "m4", 21).unwrap(); // seq 4
    mark(&s, Shown::Read, &[1, 2, 4]);
    assert_eq!(cursor(&s), 4);
    assert_eq!(remembered(&s), 0);
}

#[test]
fn a_full_read_after_a_capped_one_takes_the_rest_once_and_clears_the_record() {
    let s = room_of(6);
    mark(&s, Shown::Read, &[6, 1]);
    let rest = s.read(Seat::new("r", "alice"), true, 60).unwrap();
    assert_eq!(
        rest.iter().map(|m| m.seq).collect::<Vec<_>>(),
        vec![2, 3, 4, 5]
    );
    assert_eq!(cursor(&s), 6);
    assert_eq!(remembered(&s), 0);
    assert!(unread(&s).is_empty());
}

#[test]
fn an_acknowledgement_past_remembered_seqs_clears_them() {
    let s = room_of(6);
    mark(&s, Shown::Read, &[5, 6]);
    s.advance_cursor(Seat::new("r", "alice"), Seq(6), 60)
        .unwrap();
    assert_eq!(cursor(&s), 6);
    // forgotten with the next mark, and never returned meanwhile
    mark(&s, Shown::Read, &[]);
    assert_eq!(remembered(&s), 0);
    assert!(unread(&s).is_empty());
}

#[test]
fn the_hook_does_not_repeat_what_a_capped_read_showed_out_of_order() {
    let s = room_of(6);
    mark(&s, Shown::Read, &[6, 5]);
    let seen: Vec<i64> = s
        .hook_read(Seat::new("r", "alice"), 70)
        .unwrap()
        .iter()
        .map(|m| m.seq)
        .collect();
    assert_eq!(seen, vec![1, 2, 3, 4], "5 and 6 were shown by the read");
    assert!(s.hook_read(Seat::new("r", "alice"), 71).unwrap().is_empty());
    // the read cursor is untouched by the hook: 1 to 4 can still be read once
    assert_eq!(unread(&s), vec![1, 2, 3, 4]);
}

#[test]
fn a_wait_position_moves_over_a_shown_run_and_remembers_the_rest_without_touching_read() {
    let s = room_of(8);
    let wait_poll = |s: &SqliteStore| -> Vec<i64> {
        s.wait_poll(Seat::new("r", "alice"), 80)
            .unwrap()
            .iter()
            .map(|m| m.seq)
            .collect()
    };
    // a peeking wait looked at all eight and showed 8 and 7 first; 1 to 6 were held back
    mark(&s, Shown::Wait, &[8, 7]);
    assert_eq!(wait_poll(&s), vec![1, 2, 3, 4, 5, 6]);
    mark(&s, Shown::Wait, &[1, 2, 3, 4, 5, 6]);
    assert!(wait_poll(&s).is_empty());
    assert_eq!(cursor(&s), 0, "nothing was marked read");
    assert_eq!(unread(&s), (1..=8).collect::<Vec<_>>());
    assert_eq!(remembered(&s), 0, "the wait position passed every seq");
}

#[test]
fn what_a_read_showed_out_of_order_counts_for_the_wait_position_too() {
    let s = room_of(6);
    mark(&s, Shown::Read, &[6]);
    let polled: Vec<i64> = s
        .wait_poll(Seat::new("r", "alice"), 80)
        .unwrap()
        .iter()
        .map(|m| m.seq)
        .collect();
    assert_eq!(polled, vec![1, 2, 3, 4, 5]);
    mark(&s, Shown::Wait, &[1, 2, 3, 4, 5]);
    assert!(s.wait_poll(Seat::new("r", "alice"), 81).unwrap().is_empty());
}

#[test]
fn random_calls_of_any_size_lose_no_message_and_show_none_twice() {
    // a deterministic pseudo-random walk: each call shows a random subset of what is unread, and a few messages arrive
    let s = room_of(60);
    let mut state: u64 = 0x9e3779b97f4a7c15;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut shown: Vec<i64> = Vec::new();
    let mut posted = 60;
    for round in 0..200 {
        let pending = unread(&s);
        if pending.is_empty() && round > 150 {
            break;
        }
        let take: Vec<i64> = pending
            .iter()
            .copied()
            .filter(|_| next() % 3 == 0)
            .collect();
        for t in &take {
            assert!(!shown.contains(t), "seq {t} shown twice");
        }
        shown.extend(&take);
        mark(&s, Shown::Read, &take);
        assert_eq!(
            unread(&s).len(),
            pending.len() - take.len(),
            "the unread count falls by exactly what was shown"
        );
        if round % 7 == 0 && posted < 90 {
            posted += 1;
            s.post(Seat::new("r", "bob"), "later", 100 + posted)
                .unwrap();
        }
    }
    // finish: everything left is returned by one read, once
    let rest: Vec<i64> = s
        .read(Seat::new("r", "alice"), true, 999)
        .unwrap()
        .iter()
        .map(|m| m.seq)
        .collect();
    for t in &rest {
        assert!(!shown.contains(t), "seq {t} shown twice");
    }
    shown.extend(rest);
    shown.sort_unstable();
    assert_eq!(shown, (1..=posted).collect::<Vec<_>>(), "none lost");
    assert_eq!(remembered(&s), 0);
}
