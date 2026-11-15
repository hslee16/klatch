//! What a sender can learn about each recipient of one message (#546). Every state is derived from what members
//! already see of each other in `who` (last seen, parked), compared with the message time; none comes from a read
//! cursor or a hook shown-mark. Those say more about a member than `who` does, and whether a sender may see them is
//! a human decision (DESIGN item 61). No state is proof that a model read the message, and none carries its text.

use crate::domain::parked::span;
use crate::domain::{MemberInfo, Message};

/// One recipient's state for a message, from liveness alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Receipt {
    /// Parked (a declared long wait) with the return time ahead.
    Parked { back_by: i64 },
    /// Seen active this long after the post. Liveness: not proof the message was shown or read.
    SeenAfter { secs: i64 },
    /// Not seen since the post; last seen this long before it.
    NotSeen { secs_before: i64 },
}

impl Receipt {
    /// One line of text, no message content.
    pub fn phrase(&self, now: i64) -> String {
        match self {
            Receipt::Parked { back_by } => format!("parked, back in {}", span(back_by - now)),
            Receipt::SeenAfter { secs } => format!(
                "seen {} after the post (liveness only; not proof it was shown or read)",
                span(*secs)
            ),
            Receipt::NotSeen { secs_before } => format!(
                "not seen since the post (last seen {} before it)",
                span(*secs_before)
            ),
        }
    }
}

/// The recipient's state, or `None` for a member who is not a recipient: the sender, and anyone who joined after the
/// post (a late joiner can read history, but the message was not sent to them). A joined time of 0 is unknown.
pub fn receipt_of(member: &MemberInfo, message: &Message, now: i64) -> Option<Receipt> {
    if member.name == message.sender || (member.joined > 0 && member.joined > message.ts) {
        return None;
    }
    if let Some(p) = member.parked.as_ref().filter(|p| p.holds(now)) {
        return Some(Receipt::Parked { back_by: p.back_by });
    }
    if member.last_seen >= message.ts {
        Some(Receipt::SeenAfter {
            secs: member.last_seen - message.ts,
        })
    } else {
        Some(Receipt::NotSeen {
            secs_before: message.ts - member.last_seen,
        })
    }
}

/// The states of a message for each recipient, in member order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipts {
    pub seq: i64,
    pub posted: i64,
    pub rows: Vec<(String, Receipt)>,
}

pub fn receipts_of(members: &[MemberInfo], message: &Message, now: i64) -> Receipts {
    Receipts {
        seq: message.seq,
        posted: message.ts,
        rows: members
            .iter()
            .filter_map(|m| receipt_of(m, message, now).map(|r| (m.name.clone(), r)))
            .collect(),
    }
}

/// The text `klatch receipt` prints.
pub fn render_receipts(r: &Receipts, now: i64) -> String {
    let mut out = format!(
        "message #{} posted {} ago; recipients:\n",
        r.seq,
        span(now - r.posted)
    );
    for (name, state) in &r.rows {
        out.push_str(&format!("  {name}: {}\n", state.phrase(now)));
    }
    if r.rows.is_empty() {
        out.push_str("  (nobody else was in the room)\n");
    }
    out.push_str("shown and read state are not reported: they are not shared between members (DESIGN item 61)\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Parked;

    fn member(name: &str, joined: i64, last_seen: i64) -> MemberInfo {
        MemberInfo {
            name: name.into(),
            joined,
            last_seen,
            status: None,
            parent: None,
            parked: None,
        }
    }

    fn message(sender: &str, ts: i64) -> Message {
        Message {
            id: "ID".into(),
            seq: 4,
            sender: sender.into(),
            attribution: crate::domain::Attribution::Unattributed,
            label: "public".into(),
            body: "secret".into(),
            ts,
            kind: None,
            payload: None,
        }
    }

    #[test]
    fn the_sender_and_a_late_joiner_are_not_recipients() {
        let m = message("alice", 100);
        assert_eq!(receipt_of(&member("alice", 1, 500), &m, 600), None);
        assert_eq!(receipt_of(&member("bob", 101, 500), &m, 600), None);
        assert!(receipt_of(&member("bob", 100, 500), &m, 600).is_some());
        assert!(
            receipt_of(&member("bob", 0, 500), &m, 600).is_some(),
            "an unknown join time is not a reason to hide a recipient"
        );
    }

    #[test]
    fn parked_wins_over_liveness_until_its_return_time_passes() {
        let m = message("alice", 100);
        let mut bob = member("bob", 1, 500);
        bob.parked = Some(Parked {
            why: "ci".into(),
            back_by: 700,
        });
        assert_eq!(
            receipt_of(&bob, &m, 600),
            Some(Receipt::Parked { back_by: 700 })
        );
        assert_eq!(
            receipt_of(&bob, &m, 700),
            Some(Receipt::SeenAfter { secs: 400 })
        );
    }

    #[test]
    fn the_text_has_no_message_content_and_says_what_it_cannot_say() {
        let m = message("alice", 100);
        let rows = vec![member("bob", 1, 130), member("carol", 1, 40)];
        let out = render_receipts(&receipts_of(&rows, &m, 400), 400);
        assert!(!out.contains("secret"), "{out}");
        assert!(out.contains("bob: seen 0m after the post"), "{out}");
        assert!(out.contains("carol: not seen since the post"), "{out}");
        assert!(out.contains("not proof it was shown or read"), "{out}");
        assert!(
            out.contains("shown and read state are not reported"),
            "{out}"
        );
    }
}
