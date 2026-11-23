//! The hook digest: unread messages, claim changes and the backlog for every room a member joined.

use super::*;

impl<S: Store> Gateway<S> {
    /// The messages a hook shows for a room: all of them, or only those that mention the caller when it muted the
    /// room. A hook shows each message once and leaves the read cursor alone, so a muted-out message stays readable.
    pub(super) fn digest_messages(
        &self,
        who: &Principal,
        room: &str,
        now: i64,
    ) -> Result<(Vec<Message>, Hidden)> {
        self.sweep_ask_routes(room);
        let messages = self.store.hook_read(Seat::new(room, &who.member), now)?;
        let outside = self.hidden_count(who, room, messages.clone())?;
        let messages = self.in_view(who, room, messages)?;
        let messages = self.mark_advisory(room, messages)?;
        if !self.store.is_muted(Seat::new(room, &who.member))? {
            return Ok((messages, outside));
        }
        // a mute holds back what is not addressed to the member; an ask routed to it is addressed (#1407)
        let routed = self.routed_asks(room, &who.member)?;
        Ok((
            messages
                .into_iter()
                .filter(|m| {
                    crate::domain::mentions(&m.body, &who.member) || routed.contains(&m.seq)
                })
                .collect(),
            outside,
        ))
    }

    /// The `seq`s of `messages` that are addressed to the caller: it is mentioned, the message is a broadcast (kind `broadcast`, or `@all`), or the room is a direct message with
    /// the caller in it. A busy hook digest keeps these before newer messages (#185).
    pub(super) fn addressed_seqs(
        &self,
        who: &Principal,
        room: &str,
        messages: &[Message],
    ) -> Result<Vec<i64>> {
        let dm = self.dm_pair(room)?.is_some();
        let routed = self.routed_asks(room, &who.member)?;
        Ok(messages
            .iter()
            .filter(|m| {
                dm || crate::domain::mentions(&m.body, &who.member)
                    || super::super::delivery::is_broadcast(m)
                    || routed.contains(&m.seq)
            })
            .map(|m| m.seq)
            .collect())
    }

    /// The stop-waiting notes derived from the room as it is now (#789): see `domain::derived_asks_notes`. Nothing is
    /// stored, so the same lines come at every digest while the condition holds. A room with no open ask has none.
    pub(super) fn asks_notes(&self, room: &str, me: &str, now: i64) -> Result<Vec<String>> {
        let asks = self.store.open_asks(room, now)?;
        if asks.is_empty() {
            return Ok(Vec::new());
        }
        let claims = self.store.claims(room, now)?;
        let head = self.store.head(room)?;
        let window = crate::domain::SERVED_WINDOW as i64;
        let recent = Self::without_canary(self.store.tail(
            room,
            (head - window).max(0),
            crate::domain::SERVED_WINDOW,
        )?);
        let served = crate::domain::served_by(&recent, (&asks, &claims), (me, now));
        let members = self.store.members(room)?;
        let exited = self.exited_askers(&served, &members, now)?;
        Ok(crate::domain::derived_asks_notes(
            &crate::domain::AsksView {
                asks: &asks,
                claims: &claims,
                served: &served,
                members: &members,
                exited: &exited,
                me,
                now,
                with_cycles: true,
            },
        ))
    }

    /// Unread messages and others' claims for every room the caller has joined. A room the guard refuses
    /// is reported in `refused`, never dropped silently; store failures are errors.
    pub fn digest(&self, who: &Principal) -> Result<Digest> {
        self.digest_with(who, false)
    }

    /// `digest`, optionally with the backlog of open unclaimed asks from others (for a session start).
    pub fn digest_with(&self, who: &Principal, backlog: bool) -> Result<Digest> {
        check_member(who)?;
        let now = self.now();
        let mut rooms = Vec::new();
        let mut refused = Vec::new();
        for room in self.store.rooms_of(&who.member)? {
            let op = Operation::Read {
                room: room.clone(),
                advance: true,
            };
            if let Err(e) = self.authorize_quietly(who, &op) {
                refused.push(Refused {
                    room,
                    reason: e.to_string(),
                });
                continue;
            }
            let (messages, outside_view) = self.digest_messages(who, &room, now)?;
            let addressed = self.addressed_seqs(who, &room, &messages)?;
            // asks for the member, direct messages and the like come before chatter when the digest is full (#1405)
            let ranks = self.importances(who, &room, &messages)?;
            let dm_peer = self.dm_peer(who, &room)?;
            let move_to_dm = if dm_peer.is_none() {
                self.long_exchange(who, &room, &messages)?
            } else {
                None
            };
            let changes = self.store.claim_changes(&room, &who.member, now)?;
            let stale_after = self.settings_of(&room)?.stale_secs;
            let stale = self
                .store
                .stale_asks(Seat::new(&room, &who.member), now, stale_after)?;
            let escalated = self.store.escalated_asks(&room, &who.member, now)?;
            let lapsed = self.store.lapsed_asks(&room, &who.member, now)?;
            let aged = self.store.claim_ages(&room, &who.member, now)?;
            let cycles = self.store.deadlock_notices(&room, &who.member, now)?;
            let asks_notes = self.asks_notes(&room, &who.member, now)?;
            let crossings = self.crossing_notices(&room, &who.member)?;
            let unowned_prs = if backlog {
                let mut prs = self.open_prs_of(&room, now)?;
                prs.retain(|p| p.holder.is_none());
                prs
            } else {
                Vec::new()
            };
            let backlog = if backlog {
                self.store
                    .open_asks(&room, now)?
                    .into_iter()
                    .filter(|a| a.sender != who.member && !a.claimed)
                    .collect()
            } else {
                Vec::new()
            };
            // an ask escalated to this member after it was posted is behind its cursors: list it once, like the
            // backlog (#1407)
            let mut backlog = backlog;
            let late = self.untold_escalations(&room, &who.member, 1)?;
            if !late.is_empty() {
                let told: Vec<i64> = late.iter().map(|a| a.seq).collect();
                for a in late {
                    if !backlog.iter().any(|b| b.seq == a.seq)
                        && !messages.iter().any(|m| m.seq == a.seq)
                    {
                        backlog.push(a);
                    }
                }
                self.store.mark_told(&room, &who.member, &told, 1)?;
            }
            rooms.push(RoomDigest {
                room,
                messages,
                outside_view,
                addressed,
                ranks,
                claims: changes.added,
                released: changes.released,
                stale,
                backlog,
                escalated,
                lapsed,
                aged,
                cycles,
                asks_notes,
                crossings,
                unowned_prs,
                dm_peer,
                move_to_dm,
            });
        }
        Ok(Digest {
            rooms,
            refused,
            now,
        })
    }
}
