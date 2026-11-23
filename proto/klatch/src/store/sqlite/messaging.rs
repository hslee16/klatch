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
    fn others_after(&self, room: &str, after: i64, member: &str) -> Result<Vec<Message>> {
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
        let mut rows = self.others_after(room, last, member)?;
        // what a capped read showed out of order is read, though it lies past the cursor (#1405)
        let shown = self.shown_past(room, member, &["read"], last)?;
        rows.retain(|m| !shown.contains(&m.seq));
        Ok((rows, last))
    }

    /// The seqs past `after` that a view showed out of order, on the given channels (`read`, `wait`).
    fn shown_past(
        &self,
        room: &str,
        member: &str,
        channels: &[&str],
        after: i64,
    ) -> Result<std::collections::HashSet<i64>> {
        let mut out = std::collections::HashSet::new();
        for channel in channels {
            let rows: Vec<i64> = self.rows(
                "SELECT seq FROM read_shown WHERE room=?1 AND member=?2 AND channel=?3 AND seq>?4",
                params![room, member, channel, after],
                |r| r.get(0),
            )?;
            out.extend(rows);
        }
        Ok(out)
    }

    /// Record seqs a view showed (#1405), then move the position of `channel` over the contiguous run that is now all
    /// shown, and forget the remembered seqs the positions have passed. One transaction.
    pub(super) fn do_mark_shown(
        &self,
        at: Seat,
        channel: Shown,
        seqs: &[Seq],
        now: i64,
    ) -> Result<()> {
        let Seat { room, member } = at;
        self.immediate(|| {
            self.touch(room, member, now)?;
            let (read, wait): (i64, i64) = self
                .conn
                .query_row(
                    "SELECT COALESCE(last_seq,0), wait_seq FROM members WHERE room=?1 AND name=?2",
                    params![room, member],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap_or((0, 0));
            let (word, start) = match channel {
                Shown::Read => ("read", read),
                Shown::Wait => ("wait", read.max(wait)),
            };
            // the messages this position has still to pass, in order: only those can be shown
            let pending: Vec<i64> = self.rows(
                "SELECT seq FROM messages WHERE room=?1 AND seq>?2 AND sender<>?3 AND COALESCE(kind,'')<>'archived' ORDER BY seq",
                params![room, start, member],
                |r| r.get(0),
            )?;
            let pending_set: std::collections::HashSet<i64> = pending.iter().copied().collect();
            for s in seqs.iter().filter(|s| pending_set.contains(&s.0)) {
                self.exec(
                    "INSERT OR IGNORE INTO read_shown(room,member,channel,seq) VALUES(?1,?2,?3,?4)",
                    params![room, member, word, s.0],
                )?;
            }
            // a wait looks past the read position, so what a read showed counts for it too
            let channels: &[&str] = match channel {
                Shown::Read => &["read"],
                Shown::Wait => &["read", "wait"],
            };
            let shown = self.shown_past(room, member, channels, start)?;
            let mut to = start;
            for s in &pending {
                if shown.contains(s) {
                    to = *s;
                } else {
                    break;
                }
            }
            if to > start {
                let column = match channel {
                    Shown::Read => "last_seq",
                    Shown::Wait => "wait_seq",
                };
                self.exec(
                    &format!("UPDATE members SET {column}=MAX({column},?3), last_seen=?4 WHERE room=?1 AND name=?2"),
                    params![room, member, to, now],
                )?;
            }
            self.forget_passed_shown(room, member)
        })
    }

    /// Forget the remembered shown seqs that the read cursor and the wait position have passed.
    fn forget_passed_shown(&self, room: &str, member: &str) -> Result<()> {
        self.exec(
            "DELETE FROM read_shown WHERE room=?1 AND member=?2 AND seq <= (
                 SELECT CASE channel WHEN 'read' THEN COALESCE(last_seq,0) ELSE MAX(COALESCE(last_seq,0),wait_seq) END
                 FROM members WHERE room=?1 AND name=?2)",
            params![room, member],
        )
        .map(|_| ())
    }

    pub(super) fn do_read(&self, at: Seat, advance: bool, now: i64) -> Result<Vec<Message>> {
        let Seat { room, member } = at;
        self.touch(room, member, now)?;
        let (rows, last) = self.unread_after(room, member)?;
        if advance {
            // Advance to the last message actually returned, never to the room's current maximum:
            // a message posted between the select above and this update would otherwise be skipped
            // for good. Own messages after the last returned one are simply filtered again next time.
            // what a capped read showed out of order counts as taken too (#1405)
            let beyond = self
                .shown_past(room, member, &["read"], last)?
                .into_iter()
                .max()
                .unwrap_or(0);
            let cursor = rows.last().map_or(last, |m| m.seq.max(last)).max(beyond);
            self.exec(
                "UPDATE members SET last_seq=?3, last_seen=?4 WHERE room=?1 AND name=?2",
                params![room, member, cursor, now],
            )?;
            self.forget_passed_shown(room, member)?;
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
        let mut rows = self.others_after(room, read.max(seen), member)?;
        // what a capped read or wait showed out of order is not new (#1405)
        let channels: &[&str] = if column == "wait_seq" {
            &["read", "wait"]
        } else {
            &["read"]
        };
        let shown = self.shown_past(room, member, channels, read.max(seen))?;
        rows.retain(|m| !shown.contains(&m.seq));
        Ok(rows)
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
            let mut rows = self.others_after(room, from, member)?;
            // the hook does not repeat what a capped read showed out of order (#1405); hook_seq still moves over it
            let taken = self.shown_past(room, member, &["read"], from)?;
            let newest = rows.last().map(|m| m.seq);
            rows.retain(|m| !taken.contains(&m.seq));
            if let Some(last) = newest {
                self.exec(
                    "UPDATE members SET hook_seq=?3 WHERE room=?1 AND name=?2",
                    params![room, member, last],
                )?;
            }
            Ok(rows)
        })
    }
}
