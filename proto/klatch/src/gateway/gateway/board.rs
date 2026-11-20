//! What a member is shown unprompted or on request: the board, asks, receipts and the hook digest.

use super::*;

/// What a narrowed view held back: the count and the newest seq, `None` when nothing (#673).
type Hidden = Option<(usize, i64)>;

impl<S: Store> Gateway<S> {
    /// The messages a hook shows for a room: all of them, or only those that mention the caller when it muted the
    /// room. A hook shows each message once and leaves the read cursor alone, so a muted-out message stays readable.
    pub(super) fn digest_messages(
        &self,
        who: &Principal,
        room: &str,
        now: i64,
    ) -> Result<(Vec<Message>, Hidden)> {
        let messages = self.store.hook_read(Seat::new(room, &who.member), now)?;
        let outside = self.hidden_count(who, room, messages.clone())?;
        let messages = self.in_view(who, room, messages)?;
        if !self.store.is_muted(Seat::new(room, &who.member))? {
            return Ok((messages, outside));
        }
        Ok((
            messages
                .into_iter()
                .filter(|m| crate::domain::mentions(&m.body, &who.member))
                .collect(),
            outside,
        ))
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
        // statuses and offers are read by prefix at any depth, not from a tail window (#731)
        let messages = self.store.words(room, &crate::domain::BOARD_WORDS)?;
        let statuses = crate::domain::statuses(&messages);
        let mut all = self.store.members(room)?;
        self.with_kinds(&mut all)?;
        self.with_places(&mut all)?;
        self.with_advisory(room, &mut all)?;
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
                status_ts: m.status_ts,
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
                kind: m.kind,
                place: m.place,
                advisory: m.advisory,
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
        // the task words are read by prefix at any depth, not from the tail window: a task can outlive the window
        let words = self.store.words(room, &crate::domain::TASK_WORDS)?;
        let mut tasks = crate::domain::open_tasks(&words);
        for t in &mut tasks {
            let present = members.iter().any(|m| {
                m.name == t.owner && !m.status.as_deref().is_some_and(|s| s.starts_with("done"))
            });
            t.orphaned = !present && !holders.contains(&t.owner);
        }
        Ok(crate::domain::Board {
            room: room.into(),
            tasks,
            prs: self.open_prs_of(room, now)?,
            asks: self.store.open_asks(room, now)?,
            offers: crate::domain::current_offers(&messages, now, crate::domain::OFFER_TTL_SECS)
                .into_iter()
                .filter(|o| !done.contains(&o.sender) && !absent.contains(&o.sender))
                .collect(),
            members,
            stale_members: gone.len(),
        })
    }

    /// One member's card (#251, the design item for #251): what the room board and the live claims say about `member`, or `None` when
    /// it is not listed on the board. Derived on every call, stored nowhere, and a read like the board is.
    pub fn member_card(
        &self,
        who: &Principal,
        room: &str,
        member: &str,
    ) -> Result<Option<crate::domain::Card>> {
        if !valid_name(member) {
            return Err(Error::Invalid("bad member name".into()));
        }
        let board = self.board(who, room)?;
        let now = self.now();
        let claims = self.store.claims(room, now)?;
        let scope = self.store.member_scope(member)?;
        Ok(crate::domain::card_of(
            &board,
            member,
            scope,
            (&claims, now),
        ))
    }

    /// The pull requests the log says are open (item 84), each with the holder of its live `pr:N` claim. One that no
    /// message ended and whose claim was last released with a note that says merged is dropped: the claim's note is
    /// the other way to say a PR is over.
    fn open_prs_of(&self, room: &str, now: i64) -> Result<Vec<crate::domain::OpenPr>> {
        let words = self.store.words(room, &crate::domain::PR_WORDS)?;
        let mut prs = crate::domain::open_prs(&words);
        if prs.is_empty() {
            return Ok(prs);
        }
        let claims = self.store.claims(room, now)?;
        for pr in &mut prs {
            let path = pr.path();
            pr.holder = claims
                .iter()
                .find(|c| c.path == path)
                .map(|c| c.owner.clone());
        }
        let mut open = Vec::with_capacity(prs.len());
        for pr in prs {
            let merged = pr.holder.is_none()
                && self
                    .store
                    .last_release(room, &pr.path())?
                    .is_some_and(|r| crate::domain::is_merged_note(&r.note));
            if !merged {
                open.push(pr);
            }
        }
        Ok(open)
    }

    /// Asks in a room nobody has answered or closed.
    pub fn open_asks(&self, who: &Principal, room: &str) -> Result<Vec<crate::domain::OpenAsk>> {
        self.admit(who, room, Operation::read(room, false))?;
        self.store.open_asks(room, self.now())
    }

    /// The stuck cases of a room as structured events (#271): stale asks, asks nobody capable took, cycles through asks
    /// and cycles of claim waits. A read like `open_asks`, guarded the same way (so a room above the caller's clearance
    /// refuses it), and it carries ids, member names and numbers, never message text.
    pub fn stuck(&self, who: &Principal, room: &str) -> Result<Vec<crate::domain::StuckEvent>> {
        self.admit(who, room, Operation::read(room, false))?;
        let now = self.now();
        let asks = self.store.open_asks(room, now)?;
        let claims = self.store.claims(room, now)?;
        let cycles = self.store.wait_cycles(room, now)?;
        let stale_after = self.settings_of(room)?.stale_secs;
        Ok(crate::domain::stuck_events(
            &asks,
            &claims,
            &cycles,
            now,
            stale_after,
        ))
    }

    pub fn who(&self, who: &Principal, room: &str) -> Result<Vec<MemberInfo>> {
        self.admit(who, room, Operation::Who { room: room.into() })?;
        self.settle_leases(&who.member)?;
        let mut members = self.store.members(room)?;
        self.with_kinds(&mut members)?;
        self.with_places(&mut members)?;
        self.with_modes(&mut members)?;
        self.with_advisory(room, &mut members)?;
        Ok(members)
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
        let cursors = self.store.member_cursors(room)?;
        Ok(crate::domain::receipts_of(
            &members,
            &cursors,
            &message,
            self.now(),
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
            let changes = self.store.claim_changes(&room, &who.member, now)?;
            let stale_after = self.settings_of(&room)?.stale_secs;
            let stale = self
                .store
                .stale_asks(Seat::new(&room, &who.member), now, stale_after)?;
            let escalated = self.store.escalated_asks(&room, &who.member, now)?;
            let lapsed = self.store.lapsed_asks(&room, &who.member, now)?;
            let aged = self.store.claim_ages(&room, &who.member, now)?;
            let cycles = self.store.deadlock_notices(&room, &who.member, now)?;
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
            rooms.push(RoomDigest {
                room,
                messages,
                outside_view,
                addressed,
                claims: changes.added,
                released: changes.released,
                stale,
                backlog,
                escalated,
                lapsed,
                aged,
                cycles,
                crossings,
                unowned_prs,
            });
        }
        Ok(Digest {
            rooms,
            refused,
            now,
        })
    }
}
