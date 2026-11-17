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
        self.store
            .read(Seat::new(room, &who.member), advance, self.now())
    }

    /// What a `wait` should consider: unread messages it has not yet looked at. Moves no cursor; refreshes presence.
    pub fn wait_poll(&self, who: &Principal, room: &str) -> Result<Vec<Message>> {
        self.admit(who, room, Operation::read(room, false))?;
        self.store
            .wait_poll(Seat::new(room, &who.member), self.now())
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
        let unread = self
            .store
            .wake_poll(Seat::new(room, &who.member), self.now())?;
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
        let now = self.now();
        let unread = self.store.wake_poll(Seat::new(room, &who.member), now)?;
        match unread.iter().map(|m| m.seq).max() {
            Some(last) => self
                .store
                .wake_seen(Seat::new(room, &who.member), last, now),
            None => Ok(()),
        }
    }

    /// Record that a `wait` looked at the room up to `seq`; the messages stay unread.
    pub fn wait_seen(&self, who: &Principal, room: &str, seq: i64) -> Result<()> {
        self.admit(who, room, Operation::read(room, false))?;
        self.store
            .wait_seen(Seat::new(room, &who.member), seq, self.now())
    }

    /// Messages after `after_seq`, oldest first, at most `limit`, from every sender including the caller.
    /// No cursor or presence side effects; the guard sees a `Subscribe` operation.
    pub fn tail(
        &self,
        who: &Principal,
        room: &str,
        (after_seq, limit): (i64, usize),
    ) -> Result<Vec<Message>> {
        check_member(who)?;
        check_room(room)?;
        if after_seq < 0 || limit == 0 {
            return Err(Error::Invalid("bad tail range".into()));
        }
        self.check_guard(who, &Operation::Subscribe { room: room.into() })?;
        self.store.tail(room, after_seq, limit)
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
            .advance_cursor(Seat::new(room, &who.member), seq, self.now())
    }
}
