//! What a capped view shows first (#1405): the unread messages of a room in order of importance to the caller, and the
//! record of what a view showed when that is not a prefix of the unread messages. The classes come only from structured
//! facts (who a message is addressed to, whether the room is the caller's direct message, the ask records, the role of
//! the sender), never from the words of a message: see `domain::priority`.

use super::*;
use crate::domain::{Importance, order_by_importance};
use crate::store::Shown;
use std::collections::HashSet;

impl<S: Store> Gateway<S> {
    /// The class of each of `messages` for the caller, as `(seq, class)`; a message not listed is `Rest`. Reads the
    /// ask records, the room kind and the roles of the senders; moves nothing.
    pub(super) fn importances(
        &self,
        who: &Principal,
        room: &str,
        messages: &[Message],
    ) -> Result<Vec<(i64, Importance)>> {
        let me = who.member.as_str();
        let in_dm = self.dm_peer(who, room)?.is_some();
        // asks the caller could answer, and the caller's own asks that an answer or a close points at
        let mut answerable: HashSet<i64> = HashSet::new();
        if messages
            .iter()
            .any(|m| m.sender != me && crate::domain::is_ask(&m.body))
        {
            let open = self.store.open_asks(room, self.now())?;
            for a in open.iter().filter(|a| a.sender != me) {
                if messages.iter().any(|m| m.seq == a.seq)
                    && self
                        .could_answer(who, room, me, a.seq)
                        .is_ok_and(|c| c.could())
                {
                    answerable.insert(a.seq);
                }
            }
        }
        // asks the gateway addressed to the caller (#1407): routed by could_answer and the ask's structured fields, or
        // escalated to it as a coordinator
        let routed: HashSet<i64> = self.routed_asks(room, me)?.into_iter().collect();
        let mut own_asks: HashSet<i64> = HashSet::new();
        let mut asked: HashSet<i64> = HashSet::new();
        for m in messages {
            let target = crate::domain::answer_target(&m.body)
                .or_else(|| crate::domain::closed_target(&m.body));
            if let Some(n) = target.filter(|n| *n > 0 && asked.insert(*n))
                && let Some(ask) = self.store.tail(room, n - 1, 1)?.into_iter().next()
                && ask.seq == n
                && ask.sender == me
            {
                own_asks.insert(n);
            }
        }
        let mut coordinators: HashSet<String> = HashSet::new();
        let mut checked: HashSet<&str> = HashSet::new();
        for m in messages {
            // the roles that coordinate and can post to a room: the admin agent and the integrator (a facilitator may
            // not post to a room at all)
            if checked.insert(m.sender.as_str())
                && matches!(
                    self.role_of(&m.sender),
                    Ok(Some(Role::Admin | Role::Integrator))
                )
            {
                coordinators.insert(m.sender.clone());
            }
        }
        Ok(messages
            .iter()
            .filter_map(|m| {
                let addressed =
                    crate::domain::mentions(&m.body, me) || super::delivery::is_broadcast(m);
                let is_ask = crate::domain::is_ask(&m.body);
                let target = crate::domain::answer_target(&m.body)
                    .or_else(|| crate::domain::closed_target(&m.body));
                let class = if is_ask
                    && (addressed || answerable.contains(&m.seq) || routed.contains(&m.seq))
                    || target.is_some_and(|n| own_asks.contains(&n))
                {
                    Importance::Ask
                } else if in_dm {
                    Importance::Dm
                } else if addressed || coordinators.contains(&m.sender) {
                    Importance::Addressed
                } else {
                    return None;
                };
                Some((m.seq, class))
            })
            .collect())
    }

    /// `messages` (unread ones of `room`) in the order a capped view shows them (#1405): the caller's asks, direct
    /// messages and messages to it first, the rest after, each class in sequence order, each with its class.
    pub fn rank(
        &self,
        who: &Principal,
        room: &str,
        messages: Vec<Message>,
    ) -> Result<Vec<(Importance, Message)>> {
        self.admit(who, room, Operation::read(room, false))?;
        let classes = self.importances(who, room, &messages)?;
        let class_of = |seq: i64| {
            classes
                .iter()
                .find(|(s, _)| *s == seq)
                .map_or(Importance::Rest, |(_, c)| *c)
        };
        Ok(order_by_importance(messages, &classes)
            .into_iter()
            .map(|m| (class_of(m.seq), m))
            .collect())
    }

    /// The caller's unread messages in `room`, in the order of `rank`. Moves no cursor.
    pub fn read_ranked(&self, who: &Principal, room: &str) -> Result<Vec<(Importance, Message)>> {
        let unread = self.read(who, room, false)?;
        self.rank(who, room, unread)
    }

    /// Record that a capped view showed `seqs` (#1405). The read cursor, or for a peeking or filtered wait the wait
    /// position, moves over the longest contiguous run of shown messages; those shown beyond it are remembered, so no
    /// later call repeats them, and nothing that was not named is marked. The admission is that of a read that advances
    /// (`Read`) or of a wait (`Wait`). A librarian only looks and writes nothing, as in `read_through`.
    pub fn mark_shown(
        &self,
        who: &Principal,
        room: &str,
        channel: Shown,
        seqs: &[i64],
    ) -> Result<()> {
        self.admit(who, room, Operation::read(room, channel == Shown::Read))?;
        if self.is_librarian(&who.member)? {
            return Ok(());
        }
        let at = Seat::new(room, &who.member);
        let mut shown: Vec<i64> = seqs.to_vec();
        // a canary is never shown to a reader, and must not stop the position behind it: it counts as shown when a later
        // message was
        if let Some(&top) = seqs.iter().max() {
            let unread = match channel {
                Shown::Read => self.store.read(at, false, self.now())?,
                Shown::Wait => self.store.wait_poll(at, self.now())?,
            };
            shown.extend(
                unread
                    .iter()
                    .filter(|m| {
                        m.kind.as_deref() == Some(super::tripwire::CANARY_KIND) && m.seq < top
                    })
                    .map(|m| m.seq),
            );
        }
        let seqs: Vec<Seq> = shown.into_iter().map(Seq).collect();
        self.store.mark_shown(at, channel, &seqs, self.now())
    }
}
