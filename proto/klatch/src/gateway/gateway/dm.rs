//! Direct messages (D3, ticket #43): a two-member room made on first use. Its label is the lower of the two
//! members' clearances, worked out at each call so a change of clearance applies at once. It goes through the same
//! post path as any room: same checks, audit and receipts.

use super::*;
use crate::domain::{DM_PREFIX, dm_room};

impl<S: Store> Gateway<S> {
    /// The two members of a direct-message room, if `room` is one.
    pub(super) fn dm_pair(&self, room: &str) -> Result<Option<(String, String)>> {
        let pair = self.store.policy_get(&format!("dm:{room}"))?;
        Ok(pair.and_then(|p| {
            let (a, b) = p.split_once(',')?;
            Some((a.to_string(), b.to_string()))
        }))
    }

    /// The other member of `room` when it is a direct message the gateway made and `who` is one of its two members
    /// (#1192). The name comes from the policy record `dm` wrote, and is checked again here.
    pub(super) fn dm_peer(&self, who: &Principal, room: &str) -> Result<Option<String>> {
        Ok(self.dm_pair(room)?.and_then(|(a, b)| {
            let peer = if a == who.member {
                b
            } else if b == who.member {
                a
            } else {
                return None;
            };
            valid_name(&peer).then_some(peer)
        }))
    }

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
        let pairs: Vec<(i64, &str)> = rows.iter().map(|m| (m.seq, m.sender.as_str())).collect();
        let seqs: Vec<i64> = shown.iter().map(|m| m.seq).collect();
        let Some((peer, len)) = crate::domain::crossing_exchange(&who.member, &pairs, &seqs) else {
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
        let inside = pair.iter().all(|p| members.iter().any(|m| &m.name == p));
        if len < N
            || !inside
            || !pair.iter().all(|p| valid_name(p))
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

    /// Only the two members use a direct-message room, and a name made like one that `dm` did not make is reserved.
    pub(super) fn check_dm(&self, who: &Principal, room: &str) -> Result<()> {
        match self.dm_pair(room)? {
            Some((a, b)) if who.member == a || who.member == b => Ok(()),
            Some(_) => Err(Error::Refused {
                rule: "dm-members".into(),
                reason: format!("room {room} is a direct message between two other members"),
            }),
            None if room.starts_with(DM_PREFIX) => Err(Error::Refused {
                rule: "dm-reserved".into(),
                reason: format!("room names starting {DM_PREFIX} are for direct messages"),
            }),
            None => Ok(()),
        }
    }

    /// Send `body` to member `to` in the room the two share, made on first use. The room's ceiling is the lower of
    /// their clearances; `label` may go lower. Returns the room, which both members can `read` and `wait` on.
    pub fn dm(
        &self,
        who: &Principal,
        to: &str,
        body: &str,
        label: Option<&str>,
    ) -> Result<(String, Posted)> {
        check_member(who)?;
        if !valid_name(to) || to == who.member {
            return Err(Error::Invalid(
                "a direct message goes to one other member by name".into(),
            ));
        }
        let room = dm_room(&who.member, to);
        // a facilitator's one DM (#902): the form, the state and the capability are judged before anything is made
        let facilitator = self.is_facilitator(&who.member)?;
        let created = if facilitator {
            self.check_facilitator_dm(who, to, &room, body)?
        } else {
            false
        };
        let key = format!("dm:{room}");
        if self.store.policy_get(&key)?.is_none() {
            let (a, b) = if who.member.as_str() <= to {
                (who.member.as_str(), to)
            } else {
                (to, who.member.as_str())
            };
            self.store
                .policy_set(&key, Some(&format!("{a},{b}")), self.now())?;
        }
        let posted = self.post_labeled(who, (&room, body, label))?;
        self.store.join(&room, &who.member, self.now())?;
        self.store.join(&room, to, self.now())?;
        if facilitator {
            self.audit_facilitator_dm(who, to, &room, body, created)?;
        }
        Ok((room, posted))
    }
}
