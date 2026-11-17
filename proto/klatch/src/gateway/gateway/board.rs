//! What a member is shown unprompted or on request: the board, asks, receipts and the hook digest.

use super::*;

impl<S: Store> Gateway<S> {
    /// The messages a hook shows for a room: all of them, or only those that mention the caller when it muted the
    /// room. A hook shows each message once and leaves the read cursor alone, so a muted-out message stays readable.
    pub(super) fn digest_messages(
        &self,
        who: &Principal,
        room: &str,
        now: i64,
    ) -> Result<Vec<Message>> {
        let messages = self.store.hook_read(Seat::new(room, &who.member), now)?;
        let messages = self.in_view(who, room, messages)?;
        if !self.store.is_muted(Seat::new(room, &who.member))? {
            return Ok(messages);
        }
        Ok(messages
            .into_iter()
            .filter(|m| crate::domain::mentions(&m.body, &who.member))
            .collect())
    }

    /// The `seq`s of `messages` that are addressed to the caller: it is mentioned, or the room is a direct message with
    /// the caller in it. A busy hook digest keeps these before newer messages (#185).
    pub(super) fn addressed_seqs(
        &self,
        who: &Principal,
        room: &str,
        messages: &[Message],
    ) -> Result<Vec<i64>> {
        let dm = self.dm_pair(room)?.is_some();
        Ok(messages
            .iter()
            .filter(|m| dm || crate::domain::mentions(&m.body, &who.member))
            .map(|m| m.seq)
            .collect())
    }

    /// The room's board: open asks, current offers and members with their last announced status.
    pub fn board(&self, who: &Principal, room: &str) -> Result<crate::domain::Board> {
        self.admit(who, room, Operation::read(room, false))?;
        let now = self.now();
        let messages = self.store.tail(room, 0, 5000)?;
        let statuses = crate::domain::statuses(&messages);
        let all = self.store.members(room)?;
        let done: Vec<String> = all
            .iter()
            .filter(|m| m.status.as_deref().is_some_and(|s| s.starts_with("done")))
            .map(|m| m.name.clone())
            .collect();
        let (recent, gone): (Vec<_>, Vec<_>) = all.into_iter().partition(|m| {
            now - m.last_seen <= crate::domain::MEMBER_ACTIVE_SECS
                || m.parked.as_ref().is_some_and(|p| p.holds(now))
        });
        // An offer follows its sender's presence: a member that has gone quiet is not available, and a member
        // that keeps waiting (which refreshes presence) keeps its offer with no renewal.
        let absent: Vec<String> = gone.iter().map(|m| m.name.clone()).collect();
        let members = recent
            .into_iter()
            .map(|m| crate::domain::BoardMember {
                status: m.status.clone().or_else(|| {
                    statuses
                        .iter()
                        .find(|s| s.member == m.name)
                        .map(|s| s.status.clone())
                }),
                name: m.name,
                last_seen: m.last_seen,
                parent: m.parent,
                parked: m.parked.filter(|p| p.holds(now)),
            })
            .collect::<Vec<crate::domain::BoardMember>>();
        // A task whose owner is not present (hidden, ended, pruned), not parked and holds no live claim is orphaned:
        // it stays visible with its last checkpoint for the next helper.
        let holders: Vec<String> = self
            .store
            .claims(room, now)?
            .into_iter()
            .map(|c| c.owner)
            .collect();
        let mut tasks = crate::domain::open_tasks(&messages);
        for t in &mut tasks {
            let present = members.iter().any(|m| {
                m.name == t.owner && !m.status.as_deref().is_some_and(|s| s.starts_with("done"))
            });
            t.orphaned = !present && !holders.contains(&t.owner);
        }
        Ok(crate::domain::Board {
            room: room.into(),
            tasks,
            asks: self.store.open_asks(room, now)?,
            offers: crate::domain::current_offers(&messages, now, crate::domain::OFFER_TTL_SECS)
                .into_iter()
                .filter(|o| !done.contains(&o.sender) && !absent.contains(&o.sender))
                .collect(),
            members,
            stale_members: gone.len(),
        })
    }

    /// Asks in a room nobody has answered or closed.
    pub fn open_asks(&self, who: &Principal, room: &str) -> Result<Vec<crate::domain::OpenAsk>> {
        self.admit(who, room, Operation::read(room, false))?;
        self.store.open_asks(room, self.now())
    }

    pub fn who(&self, who: &Principal, room: &str) -> Result<Vec<MemberInfo>> {
        self.admit(who, room, Operation::Who { room: room.into() })?;
        self.settle_leases(&who.member)?;
        self.store.members(room)
    }

    /// What the sender of message `seq` can learn about each recipient (#546): liveness states only (parked, seen
    /// after the post, not seen), never a cursor or a hook shown-mark, and only for the sender's own message. The
    /// text of the message is not returned.
    pub fn receipt(
        &self,
        who: &Principal,
        room: &str,
        seq: i64,
    ) -> Result<crate::domain::Receipts> {
        self.admit(who, room, Operation::Who { room: room.into() })?;
        let not_found = || Error::NotFound(format!("no message #{seq} in {room}"));
        if seq < 1 {
            return Err(not_found());
        }
        let message = self
            .store
            .tail(room, seq - 1, 1)?
            .into_iter()
            .find(|m| m.seq == seq)
            .ok_or_else(not_found)?;
        if message.sender != who.member {
            return Err(Error::Refused {
                rule: "receipt-sender-only".into(),
                reason: "only the sender of a message can ask for its receipts".into(),
            });
        }
        let members = self.store.members(room)?;
        Ok(crate::domain::receipts_of(&members, &message, self.now()))
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
            let messages = self.digest_messages(who, &room, now)?;
            let addressed = self.addressed_seqs(who, &room, &messages)?;
            let changes = self.store.claim_changes(&room, &who.member, now)?;
            let stale =
                self.store
                    .stale_asks(Seat::new(&room, &who.member), now, STALE_AFTER_SECS)?;
            let escalated = self.store.escalated_asks(&room, &who.member, now)?;
            let lapsed = self.store.lapsed_asks(&room, &who.member, now)?;
            let aged = self.store.claim_ages(&room, &who.member, now)?;
            let cycles = self.store.deadlock_notices(&room, &who.member, now)?;
            let backlog = if backlog {
                self.store
                    .open_asks(&room, now)?
                    .into_iter()
                    .filter(|a| a.sender != who.member && !a.claimed)
                    .collect()
            } else {
                Vec::new()
            };
            rooms.push(RoomDigest {
                room,
                messages,
                addressed,
                claims: changes.added,
                released: changes.released,
                stale,
                backlog,
                escalated,
                lapsed,
                aged,
                cycles,
            });
        }
        Ok(Digest {
            rooms,
            refused,
            now,
        })
    }
}
