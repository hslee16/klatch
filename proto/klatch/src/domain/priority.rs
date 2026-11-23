//! What matters first in a capped view (#1405). A read, a wait or a hook digest that cannot show every unread message
//! shows the important ones first, so a message addressed to the reader is never starved behind older chatter. The class
//! of a message comes only from structured facts about it (who it is addressed to, the kind of room, the ask records,
//! the role of the sender), never from words in its text: message text is data.
//!
//! The classes, most important first:
//!
//! 1. `Ask`: an ask addressed to the reader, an open ask the reader could answer (#1171, #671), and an answer or close
//!    that points at one of the reader's own asks.
//! 2. `Dm`: any message in a direct message room the reader is in.
//! 3. `Claim`: reserved for claim events that touch files the reader holds or waits on. A claim event is not a message in
//!    the room's log (claims are their own records), so no message has this class; the hook digest prints those events
//!    ahead of every message and never drops them (#1406), which is what this class stands for there.
//! 4. `Addressed`: any other message addressed to the reader (a mention, a broadcast), and a message from a member that
//!    holds a coordinating role (the admin agent, the facilitator), which is how decisions and handoffs reach it.
//! 5. `Rest`: everything else, in sequence order.
//!
//! Inside a class messages keep their sequence order, oldest first, so a view that has only one class is exactly the
//! chronological view it was before.

use crate::domain::Message;

/// How important a message is to its reader; the derived order is the order of showing (smaller is first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Importance {
    Ask,
    Dm,
    Claim,
    Addressed,
    Rest,
}

/// `messages` in the order of showing: by class, then by sequence. The sort is stable, so equal classes stay in the order
/// given (which is the sequence order for an unread list).
pub fn order_by_importance(messages: Vec<Message>, classes: &[(i64, Importance)]) -> Vec<Message> {
    let class_of = |m: &Message| {
        classes
            .iter()
            .find(|(seq, _)| *seq == m.seq)
            .map_or(Importance::Rest, |(_, c)| *c)
    };
    let mut keyed: Vec<(Importance, Message)> =
        messages.into_iter().map(|m| (class_of(&m), m)).collect();
    keyed.sort_by_key(|(c, m)| (*c, m.seq));
    keyed.into_iter().map(|(_, m)| m).collect()
}

/// Whether the shown order differs from the sequence order, so the view must say it is not chronological.
pub fn is_reordered(messages: &[Message]) -> bool {
    messages.windows(2).any(|w| w[0].seq > w[1].seq)
}

/// The line that says a view is in order of importance and not in sequence order. A fixed sentence.
pub const REORDERED_NOTE: &str = "[shown by importance, not in sequence: asks for you, direct messages, messages to you, then the rest; the seq of each row is exact]";

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(seq: i64) -> Message {
        Message {
            id: format!("{seq}"),
            seq,
            sender: "bob".into(),
            attribution: crate::domain::Attribution::Unattributed,
            label: "public".into(),
            body: String::new(),
            ts: 0,
            kind: None,
            payload: None,
            advisory: false,
            scope: None,
            refs: Vec::new(),
        }
    }

    #[test]
    fn the_order_is_by_class_then_by_sequence() {
        let ms: Vec<Message> = (1..=6).map(msg).collect();
        let classes = [
            (5, Importance::Ask),
            (3, Importance::Addressed),
            (6, Importance::Dm),
            (2, Importance::Ask),
        ];
        let out: Vec<i64> = order_by_importance(ms, &classes)
            .iter()
            .map(|m| m.seq)
            .collect();
        assert_eq!(out, vec![2, 5, 6, 3, 1, 4]);
    }

    #[test]
    fn without_classes_the_order_is_unchanged_and_not_reordered() {
        let ms: Vec<Message> = (1..=4).map(msg).collect();
        let out = order_by_importance(ms.clone(), &[]);
        assert_eq!(out, ms);
        assert!(!is_reordered(&out));
        assert!(is_reordered(&[msg(3), msg(1)]));
    }

    #[test]
    fn the_classes_are_ordered_from_the_most_important() {
        assert!(Importance::Ask < Importance::Dm);
        assert!(Importance::Dm < Importance::Claim);
        assert!(Importance::Claim < Importance::Addressed);
        assert!(Importance::Addressed < Importance::Rest);
    }
}
