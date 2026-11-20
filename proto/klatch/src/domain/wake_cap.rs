//! The bound on what one `klatch wait` prints for a room (#640, DESIGN item 73). A busy lobby must not flood a
//! waiting session: past `DIGEST_MAX_MESSAGES` the newest messages addressed to the member come first, then the
//! newest others, and the rest are only counted.

use crate::domain::Message;

/// What a wait prints for one room: the messages kept (oldest first, as they are read) and how many were left out.
#[derive(Debug, PartialEq, Eq)]
pub struct Capped<'a> {
    pub shown: Vec<&'a Message>,
    pub omitted: usize,
}

/// At most `max` of `messages`: those that mention `me` (newest first), then the others (newest first).
pub fn cap_wake<'a>(messages: &'a [Message], me: &str, max: usize) -> Capped<'a> {
    let newest_first = messages.iter().rev();
    let (addressed, others): (Vec<&Message>, Vec<&Message>) =
        newest_first.partition(|m| crate::domain::mentions(&m.body, me));
    let mut shown: Vec<&Message> = addressed.into_iter().chain(others).take(max).collect();
    let omitted = messages.len() - shown.len();
    shown.sort_by_key(|m| m.seq);
    Capped { shown, omitted }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(seq: i64, body: &str) -> Message {
        Message {
            id: format!("ID{seq}"),
            seq,
            sender: "bob".into(),
            attribution: crate::domain::Attribution::Unattributed,
            label: "public".into(),
            body: body.into(),
            ts: 0,
            kind: None,
            payload: None,
            advisory: false,
        }
    }

    fn seqs(c: &Capped) -> Vec<i64> {
        c.shown.iter().map(|m| m.seq).collect()
    }

    #[test]
    fn under_the_cap_everything_is_shown_and_nothing_is_omitted() {
        let ms: Vec<Message> = (1..=3).map(|i| msg(i, "x")).collect();
        let c = cap_wake(&ms, "alice", 5);
        assert_eq!(seqs(&c), vec![1, 2, 3]);
        assert_eq!(c.omitted, 0);
    }

    #[test]
    fn over_the_cap_the_newest_are_kept_and_the_rest_counted() {
        let ms: Vec<Message> = (1..=10).map(|i| msg(i, "x")).collect();
        let c = cap_wake(&ms, "alice", 4);
        assert_eq!(seqs(&c), vec![7, 8, 9, 10]);
        assert_eq!(c.omitted, 6);
    }

    #[test]
    fn addressed_messages_come_before_newer_unaddressed_ones() {
        let mut ms: Vec<Message> = (1..=10).map(|i| msg(i, "x")).collect();
        ms[1].body = "@alice look".into();
        ms[3].body = "@alice again".into();
        let c = cap_wake(&ms, "alice", 4);
        assert_eq!(seqs(&c), vec![2, 4, 9, 10], "shown oldest first");
        assert_eq!(c.omitted, 6);
    }

    #[test]
    fn more_addressed_than_the_cap_keeps_the_newest_addressed() {
        let ms: Vec<Message> = (1..=6).map(|i| msg(i, "@alice hi")).collect();
        let c = cap_wake(&ms, "alice", 2);
        assert_eq!(seqs(&c), vec![5, 6]);
        assert_eq!(c.omitted, 4);
    }
}
