//! The long exchange of two members in a shared room (#1190, #1191): the nudge to move it to a direct message, and the
//! stuck-case event for the mediator. Both count the newest run of messages sent by only two members. Nothing is
//! stored here; the direct-message room itself is `dm.rs`.

use super::*;
use crate::domain::dm_room;

impl<S: Store> Gateway<S> {
    /// Whether the caller's newly shown `shown` messages carry the Nth message of a long exchange with one peer in a
    /// shared room (#1190), and with whom: the peer and the length of the exchange.
    ///
    /// The exchange is the newest run of messages sent by only two members. It needs a room of three or more, both
    /// members in the run, and no dm between them yet. Nothing is stored: the nudge is for the digest that first shows
    /// the caller a message of the peer's at or past the threshold. A hook shows each message once, so that happens
    /// once; a third member's message ends the run, so a later exchange counts afresh. The read is the room's newest
    /// `DM_NUDGE_WINDOW` messages.
    pub(super) fn long_exchange(
        &self,
        who: &Principal,
        room: &str,
        shown: &[Message],
    ) -> Result<Option<(String, usize)>> {
        use crate::domain::DM_NUDGE_WINDOW as WINDOW;
        if shown.is_empty() || self.store.members(room)?.len() < 3 {
            return Ok(None);
        }
        let head = self.store.head(room)?;
        let after = (head - WINDOW as i64).max(0);
        let rows = Self::without_canary(self.store.tail(room, after, WINDOW)?);
        let pairs: Vec<(i64, &str, &str)> = rows
            .iter()
            .map(|m| (m.seq, m.sender.as_str(), m.body.as_str()))
            .collect();
        let seqs: Vec<i64> = shown.iter().map(|m| m.seq).collect();
        let Some((peer, len)) = crate::domain::dm_nudge(&who.member, &pairs, &seqs) else {
            return Ok(None);
        };
        if self.dm_pair(&dm_room(&who.member, &peer))?.is_some() {
            return Ok(None);
        }
        Ok(Some((peer, len)))
    }

    /// The long exchange standing in `room` as a stuck-case event for the mediator (#1191): the same newest run of
    /// messages from only two members that the digest nudge counts (`exchange_run`), at or past the threshold, in a room
    /// of three or more whose two members are both still in it and have no dm yet. Ids, names and numbers only.
    pub(super) fn exchange_event(&self, room: &str) -> Result<Option<crate::domain::StuckEvent>> {
        use crate::domain::{DM_NUDGE_MESSAGES as N, DM_NUDGE_WINDOW as WINDOW};
        let members = self.store.members(room)?;
        if members.len() < 3 {
            return Ok(None);
        }
        let head = self.store.head(room)?;
        let after = (head - WINDOW as i64).max(0);
        let rows = Self::without_canary(self.store.tail(room, after, WINDOW)?);
        let senders: Vec<&str> = rows.iter().map(|m| m.sender.as_str()).collect();
        let Some((last, other, len)) = crate::domain::exchange_run(&senders) else {
            return Ok(None);
        };
        let mut pair = vec![last.to_string(), other.to_string()];
        pair.sort();
        if len < N || !both_in(&pair, &members) {
            return Ok(None);
        }
        if !pair.iter().all(|p| valid_name(p))
            || self.dm_pair(&dm_room(&pair[0], &pair[1]))?.is_some()
        {
            return Ok(None);
        }
        Ok(Some(crate::domain::StuckEvent::LongExchange {
            room: room.to_string(),
            members: pair,
            messages: len,
        }))
    }
}

/// Whether both members of an exchange are still in the room.
fn both_in(pair: &[String], members: &[MemberInfo]) -> bool {
    pair.iter().all(|p| members.iter().any(|m| &m.name == p))
}
