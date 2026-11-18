//! Posting and the reads that move a member's cursors.

use super::*;

impl SqliteStore {
    pub(super) fn do_post_as(&self, message: &NewMessage, now: i64) -> Result<Posted> {
        let NewMessage {
            room,
            sender,
            attribution,
            label,
            body,
            ..
        } = *message;
        self.immediate(|| {
            let seq: i64 = self
                .conn
                .query_row(
                    "SELECT COALESCE(MAX(seq),0)+1 FROM messages WHERE room=?1",
                    params![room],
                    |r| r.get(0),
                )
                .map_err(be)?;
            let id = new_id((now.max(0) as u64) * 1000);
            let stored = self.seal_text((label, &id, "body"), body)?;
            let payload = match message.payload {
                Some(p) => Some(self.seal_text((label, &id, "payload"), p)?),
                None => None,
            };
            self.exec(
                    "INSERT INTO messages(room,seq,mid,sender,member,body,ts,attribution,label,kind,payload) VALUES(?1,?2,?3,?4,?4,?5,?6,?7,?8,?9,?10)",
                    params![room, seq, id, sender, stored, now, attribution_text(attribution), label, message.kind, payload],
                )?;
            self.touch(room, sender, now)?;
            Ok(Posted { id, seq })
        })
    }

    /// The live messages in the room after `after` that the member did not send, in order.
    fn from_others_after(&self, room: &str, after: i64, member: &str) -> Result<Vec<Message>> {
        self.rows(&format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE room=?1 AND seq>?2 AND sender<>?3 AND COALESCE(kind,'')<>'archived' ORDER BY seq"), params![room, after, member], |r| self.open_message(r))
    }

    /// The messages from others past the member's read cursor, with that cursor. Touches nothing: a member that has
    /// no row has read nothing.
    pub(super) fn unread_after(&self, room: &str, member: &str) -> Result<(Vec<Message>, i64)> {
        let last: i64 = self
            .conn
            .query_row(
                "SELECT last_seq FROM members WHERE room=?1 AND name=?2",
                params![room, member],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let rows = self.from_others_after(room, last, member)?;
        Ok((rows, last))
    }

    pub(super) fn do_read(&self, at: Seat, advance: bool, now: i64) -> Result<Vec<Message>> {
        let Seat { room, member } = at;
        self.touch(room, member, now)?;
        let (rows, last) = self.unread_after(room, member)?;
        if advance {
            // Advance to the last message actually returned, never to the room's current maximum:
            // a message posted between the select above and this update would otherwise be skipped
            // for good. Own messages after the last returned one are simply filtered again next time.
            let cursor = rows.last().map_or(last, |m| m.seq.max(last));
            self.exec(
                "UPDATE members SET last_seq=?3, last_seen=?4 WHERE room=?1 AND name=?2",
                params![room, member, cursor, now],
            )?;
        }
        Ok(rows)
    }

    pub(super) fn do_wait_poll(&self, room: &str, member: &str, now: i64) -> Result<Vec<Message>> {
        // A waiting member is present: every poll refreshes presence, as a read does (#237).
        self.poll_past(("wait_seq", room, member), now)
    }

    pub(super) fn do_wake_poll(&self, room: &str, member: &str, now: i64) -> Result<Vec<Message>> {
        self.poll_past(("wake_seq", room, member), now)
    }

    /// Refresh the member's presence, then the messages from others past both its read cursor and the position kept
    /// in `column` (`wait_seq` or `wake_seq`, never caller text). Moves no cursor.
    fn poll_past(
        &self,
        (column, room, member): (&str, &str, &str),
        now: i64,
    ) -> Result<Vec<Message>> {
        self.touch(room, member, now)?;
        let (read, seen): (i64, i64) = self
            .conn
            .query_row(
                &format!(
                    "SELECT COALESCE(last_seq,0), {column} FROM members WHERE room=?1 AND name=?2"
                ),
                params![room, member],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or((0, 0));
        self.from_others_after(room, read.max(seen), member)
    }

    /// True when a hook for the member would show nothing and write nothing: it was already seen this second, no claim
    /// of its is due a renewal and no message waits. A read only, so the hook can skip the writer lock (#128).
    fn hook_is_idle(&self, room: &str, member: &str, now: i64) -> Result<bool> {
        if self.last_seen(room, member)? != Some(now)
            || !self.due_renewals(room, member, now)?.is_empty()
        {
            return Ok(false);
        }
        let waiting: bool = self
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM messages m, members s WHERE s.room=?1 AND s.name=?2
                     AND m.room=?1 AND m.seq>MAX(COALESCE(s.last_seq,0),s.hook_seq)
                     AND m.sender<>?2 AND COALESCE(m.kind,'')<>'archived')",
                params![room, member],
                |r| r.get(0),
            )
            .map_err(be)?;
        Ok(!waiting)
    }

    pub(super) fn do_hook_read(&self, room: &str, member: &str, now: i64) -> Result<Vec<Message>> {
        // The usual hook finds nothing new; that must not queue behind writers.
        if self.snapshot(|| self.hook_is_idle(room, member, now))? {
            return Ok(Vec::new());
        }
        self.immediate(|| {
            self.touch(room, member, now)?;
            let (read, shown): (i64, i64) = self
                .conn
                .query_row(
                    "SELECT COALESCE(last_seq,0), hook_seq FROM members WHERE room=?1 AND name=?2",
                    params![room, member],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(be)?;
            let from = read.max(shown);
            let rows = self.from_others_after(room, from, member)?;
            if let Some(last) = rows.last() {
                self.exec(
                    "UPDATE members SET hook_seq=?3 WHERE room=?1 AND name=?2",
                    params![room, member, last.seq],
                )?;
            }
            Ok(rows)
        })
    }
}
