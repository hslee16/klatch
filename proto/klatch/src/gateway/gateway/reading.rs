//! Reading and waiting on a room: cursors, wake counts and the raw history of messages.

use super::*;

impl<S: Store> Gateway<S> {
    /// Mute or unmute a room for the caller: its digest then shows only messages that mention it (#20). An explicit
    /// `read` still returns everything.
    pub fn set_muted(&self, who: &Principal, room: &str, muted: bool) -> Result<()> {
        check_member(who)?;
        check_room(room)?;
        self.authorize(
            who,
            &Operation::Mute {
                room: room.into(),
                muted,
            },
        )?;
        self.store
            .set_muted(Seat::new(room, &who.member), muted, self.now())
    }

    pub fn read(&self, who: &Principal, room: &str, advance: bool) -> Result<Vec<Message>> {
        self.admit(who, room, Operation::read(room, advance))?;
        let at = Seat::new(room, &who.member);
        // a librarian only looks: its peek must not refresh its presence or make it a member (#499)
        if !advance && self.is_librarian(&who.member)? {
            let seen = Self::without_canary(self.store.look(at, self.now())?);
            return self.mark_advisory(room, seen);
        }
        let seen = Self::without_canary(self.store.read(at, advance, self.now())?);
        self.mark_advisory(room, seen)
    }

    /// What a librarian's peek returns without touching presence, membership or any position (#499, #750).
    fn peek_quietly(&self, who: &Principal, room: &str) -> Result<Vec<Message>> {
        let seen = Self::without_canary(self.store.look(Seat::new(room, &who.member), self.now())?);
        self.mark_advisory(room, seen)
    }

    /// What a `wait` should consider: unread messages it has not yet looked at. Moves no cursor; refreshes presence.
    pub fn wait_poll(&self, who: &Principal, room: &str) -> Result<Vec<Message>> {
        self.admit(who, room, Operation::read(room, false))?;
        if self.is_librarian(&who.member)? {
            return self.peek_quietly(who, room);
        }
        let seen = Self::without_canary(
            self.store
                .wait_poll(Seat::new(room, &who.member), self.now())?,
        );
        self.mark_advisory(room, seen)
    }

    /// How many unread messages would wake the caller in a room: from others, past both cursors, and only those
    /// that mention the caller when it muted the room. Moves no cursor; refreshes presence, so a session that sits
    /// in a watcher is not taken for gone (#37, #39).
    pub fn wakeable(&self, who: &Principal, room: &str) -> Result<usize> {
        Ok(self.wake_summary(who, room)?.0)
    }

    /// `wakeable` plus the newest wakeable seq (0 when none), so a watcher can word a pointer that differs every
    /// time (Claude Code drops a nudge identical to a recent one, #550).
    pub fn wake_summary(&self, who: &Principal, room: &str) -> Result<(usize, i64)> {
        self.admit(who, room, Operation::read(room, false))?;
        let unread = if self.is_librarian(&who.member)? {
            self.store.look(Seat::new(room, &who.member), self.now())?
        } else {
            self.store
                .wake_poll(Seat::new(room, &who.member), self.now())?
        };
        let unread = Self::without_canary(unread);
        let unread = self.in_view(who, room, unread)?;
        let muted = self.store.is_muted(Seat::new(room, &who.member))?;
        let counted = unread
            .iter()
            .filter(|m| !muted || crate::domain::mentions(&m.body, &who.member));
        Ok(counted.fold((0, 0), |(n, last), m| (n + 1, last.max(m.seq))))
    }

    /// Record that a watcher pointed at everything unread in the room, so it does not wake the caller for those
    /// again. The messages stay unread, and `wait` and `read` still see them.
    pub fn wake_mark(&self, who: &Principal, room: &str) -> Result<()> {
        self.admit(who, room, Operation::read(room, false))?;
        // a librarian writes no position and refreshes no presence (#750)
        if self.is_librarian(&who.member)? {
            return Ok(());
        }
        let now = self.now();
        let unread = self.store.wake_poll(Seat::new(room, &who.member), now)?;
        match unread.iter().map(|m| m.seq).max() {
            Some(last) => self
                .store
                .wake_seen(Seat::new(room, &who.member), Seq(last), now),
            None => Ok(()),
        }
    }

    /// Record that a `wait` looked at the room up to `seq`; the messages stay unread.
    pub fn wait_seen(&self, who: &Principal, room: &str, seq: i64) -> Result<()> {
        self.admit(who, room, Operation::read(room, false))?;
        if self.is_librarian(&who.member)? {
            return Ok(());
        }
        self.store
            .wait_seen(Seat::new(room, &who.member), Seq(seq), self.now())
    }

    /// Messages after `after_seq`, oldest first, at most `limit`, from every sender including the caller.
    /// No cursor or presence side effects; the guard sees a `Subscribe` operation.
    pub fn tail(&self, who: &Principal, room: &str, range: (i64, usize)) -> Result<Vec<Message>> {
        Ok(self.tail_scan(who, room, range)?.0)
    }

    /// `tail`, plus how far the scan went: the number of rows the store returned and the highest `seq` among them
    /// (`after_seq` when none). A reader that pages by `seq` must advance by this, not by the last message shown, or a
    /// page that held only a canary (#954) looks empty and the reader never moves on.
    pub(crate) fn tail_scan(
        &self,
        who: &Principal,
        room: &str,
        (after_seq, limit): (i64, usize),
    ) -> Result<(Vec<Message>, usize, i64)> {
        check_member(who)?;
        check_room(room)?;
        if after_seq < 0 || limit == 0 {
            return Err(Error::Invalid("bad tail range".into()));
        }
        self.check_guard(who, &Operation::Subscribe { room: room.into() })?;
        let rows = self.store.tail(room, after_seq, limit)?;
        let (scanned, last) = (rows.len(), rows.last().map_or(after_seq, |m| m.seq));
        Ok((Self::without_canary(rows), scanned, last))
    }

    /// The store's change token (#616): differs after any other connection commits, says nothing about what. It is no
    /// read, so it asks nothing of the guard; a wake still goes through the checked poll.
    pub fn change_token(&self) -> Option<u64> {
        self.store.change_token()
    }

    /// The room's highest `seq`, 0 when empty; the guard sees a `Subscribe` operation.
    pub fn head(&self, who: &Principal, room: &str) -> Result<i64> {
        self.admit(who, room, Operation::Subscribe { room: room.into() })?;
        self.store.head(room)
    }

    /// Acknowledge messages up to `seq`: moves the caller's read cursor forward, never past the head.
    pub fn ack(&self, who: &Principal, room: &str, seq: i64) -> Result<()> {
        check_member(who)?;
        check_room(room)?;
        if seq < 0 {
            return Err(Error::Invalid("bad acknowledgement".into()));
        }
        self.check_guard(who, &Operation::Ack { room: room.into() })?;
        self.store
            .advance_cursor(Seat::new(room, &who.member), Seq(seq), self.now())
    }
}
