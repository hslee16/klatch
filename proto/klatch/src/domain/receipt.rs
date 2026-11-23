//! What a sender can learn about each recipient of one message (#546, #675). The liveness state is derived from what
//! members already see of each other in `who` (last seen, parked), compared with the message time. The `shown` and
//! `read` flags come from the hook shown-mark and the read cursor; they say more about a member than `who` does, so
//! the gateway gives them to the sender of the message only (decided on #585, DESIGN item 61). A transport write
//! never counts (#555). No state carries message text.

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

/// Whether the member joined (a known time) after the message was posted.
fn joined_after(member: &MemberInfo, message: &Message) -> bool {
    member.joined > 0 && member.joined > message.ts
}

/// The recipient's state, or `None` for a member who is not a recipient: the sender, and anyone who joined after the
/// post (a late joiner can read history, but the message was not sent to them). A joined time of 0 is unknown.
pub fn receipt_of(member: &MemberInfo, message: &Message, now: i64) -> Option<Receipt> {
    if member.name == message.sender || joined_after(member, message) {
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

/// One member's cursors in a room, as the store keeps them: the read cursor, the hook shown-mark and whether the
/// room is muted for the member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberCursor {
    pub name: String,
    pub read: i64,
    pub hook: i64,
    /// The furthest a `wait` or a wake has looked (#1411): messages it returned were shown to the member even though
    /// no read moved the cursor.
    pub polled: i64,
    pub muted: bool,
}

/// Whether a recipient was shown a message by its hook, and whether it has read it past its read cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Taken {
    pub shown: bool,
    pub read: bool,
}

/// `shown`: the hook mark reached the message and, for a muted room, the message mentions the member (a muted hook
/// shows only mentions, so the mark passing a message does not mean it was shown). `read`: the read cursor reached it.
pub fn taken_of(cursor: &MemberCursor, message: &Message) -> Taken {
    let visible = !cursor.muted || crate::domain::mentions(&message.body, &cursor.name);
    Taken {
        shown: cursor.hook >= message.seq && visible,
        read: cursor.read >= message.seq,
    }
}

/// The states of a message for each recipient, in member order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipts {
    pub seq: i64,
    pub posted: i64,
    pub rows: Vec<(String, Receipt)>,
    /// Shown and read per recipient, by name. A recipient missing here has no cursor the store could report.
    pub taken: Vec<(String, Taken)>,
}

pub fn receipts_of(
    members: &[MemberInfo],
    cursors: &[MemberCursor],
    message: &Message,
    now: i64,
) -> Receipts {
    let rows: Vec<(String, Receipt)> = members
        .iter()
        .filter_map(|m| receipt_of(m, message, now).map(|r| (m.name.clone(), r)))
        .collect();
    let taken = rows
        .iter()
        .filter_map(|(name, _)| {
            let c = cursors.iter().find(|c| &c.name == name)?;
            Some((name.clone(), taken_of(c, message)))
        })
        .collect();
    Receipts {
        seq: message.seq,
        posted: message.ts,
        rows,
        taken,
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
        out.push_str(&format!("  {name}: {}", state.phrase(now)));
        if let Some((_, t)) = r.taken.iter().find(|(n, _)| n == name) {
            let yn = |b: bool| if b { "yes" } else { "no" };
            out.push_str(&format!("; shown: {}, read: {}", yn(t.shown), yn(t.read)));
        }
        out.push('\n');
    }
    if r.rows.is_empty() {
        out.push_str("  (nobody else was in the room)\n");
    }
    out.push_str("shown: the recipient's hook showed it (a muted room shows only mentions); read: its read cursor reached it. Only you, the sender, can see this (DESIGN item 61)\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Parked;

    fn member(name: &str, joined: i64, last_seen: i64) -> MemberInfo {
        MemberInfo {
            status_ts: 0,
            name: name.into(),
            joined,
            last_seen,
            status: None,
            parent: None,
            parked: None,
            kind: None,
            place: None,
            mode: None,
            advisory: false,
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
            advisory: false,
            scope: None,
            refs: Vec::new(),
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
        let cursors = vec![MemberCursor {
            name: "bob".into(),
            read: 0,
            hook: 4,
            polled: 0,
            muted: false,
        }];
        let out = render_receipts(&receipts_of(&rows, &cursors, &m, 400), 400);
        assert!(!out.contains("secret"), "{out}");
        assert!(out.contains("bob: seen 0m after the post"), "{out}");
        assert!(out.contains("carol: not seen since the post"), "{out}");
        assert!(out.contains("not proof it was shown or read"), "{out}");
        assert!(
            out.contains("bob: seen 0m after the post (liveness only; not proof it was shown or read); shown: yes, read: no"),
            "{out}"
        );
        let carol = out.lines().find(|l| l.contains("carol:")).unwrap();
        assert!(!carol.contains("shown"), "no cursor, no claim: {out}");
    }

    fn cursor(read: i64, hook: i64, muted: bool) -> MemberCursor {
        MemberCursor {
            name: "bob".into(),
            read,
            hook,
            polled: 0,
            muted,
        }
    }

    #[test]
    fn shown_follows_the_hook_mark_and_read_follows_the_read_cursor() {
        let m = message("alice", 100); // seq 4
        let t = |c| taken_of(&c, &m);
        let none = Taken {
            shown: false,
            read: false,
        };
        assert_eq!(t(cursor(0, 0, false)), none);
        assert_eq!(t(cursor(0, 3, false)), none);
        assert_eq!(
            t(cursor(0, 4, false)),
            Taken {
                shown: true,
                read: false
            }
        );
        assert_eq!(
            t(cursor(4, 0, false)),
            Taken {
                shown: false,
                read: true
            }
        );
        assert_eq!(
            t(cursor(9, 9, false)),
            Taken {
                shown: true,
                read: true
            }
        );
    }

    #[test]
    fn a_muted_room_counts_as_shown_only_for_a_mention() {
        let mut m = message("alice", 100);
        assert!(
            !taken_of(&cursor(0, 9, true), &m).shown,
            "muted, no mention"
        );
        m.body = "ping @bob".into();
        assert!(taken_of(&cursor(0, 9, true), &m).shown, "muted, mentioned");
        assert!(
            !taken_of(&cursor(0, 3, true), &m).shown,
            "mark not there yet"
        );
    }
}
