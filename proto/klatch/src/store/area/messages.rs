//! The `MessageStore` area of the store seam; `Store` in the parent module combines every area.

use crate::domain::{Attribution, Message, Posted, Result};
use crate::store::{NewMessage, Seat};

pub trait MessageStore {
    /// Join a room (idempotent). Unread starts at the beginning of history.
    fn join(&self, room: &str, member: &str, now: i64) -> Result<()>;

    /// Append a message, recording whether its sender was proven and the label it is posted under; also marks
    /// the sender as present. Returns its unique id and its per-room `seq` (dense, starting at 1).
    fn post_as(&self, message: &NewMessage, now: i64) -> Result<Posted>;

    /// `post_as` for a sender that is not proven, under the lowest label: the cautious default. The gateway
    /// never uses this; it passes the caller's real attribution.
    fn post(&self, at: Seat, body: &str, now: i64) -> Result<Posted> {
        let message = NewMessage::new(
            (at.room, at.member),
            Attribution::Unattributed,
            "public",
            body,
        );
        self.post_as(&message, now)
    }

    /// Messages from others after the member's cursor. `advance` moves the cursor to the room's end.
    fn read(&self, at: Seat, advance: bool, now: i64) -> Result<Vec<Message>>;

    /// The times of `member`'s posts at or after `since`, in every room, oldest first (the rate limit, #187). The
    /// default knows none.
    fn posts_since(&self, _member: &str, _since: i64) -> Result<Vec<i64>> {
        Ok(Vec::new())
    }

    /// Messages from others past both the read cursor and the member's wait position. Changes no cursor, but
    /// refreshes the member's presence, as a read does.
    /// The default is `read` without advancing, which has no wait position.
    fn wait_poll(&self, at: Seat, now: i64) -> Result<Vec<Message>> {
        self.read(at, false, now)
    }

    /// What `read(at, false, now)` returns, for a reader that only looks (the librarian, #499): it must not refresh
    /// the reader's presence, join it to the room or move a cursor. The default is a non-advancing `read`, which
    /// refreshes presence; a store that can look without touching anything overrides it.
    fn look(&self, at: Seat, now: i64) -> Result<Vec<Message>> {
        self.read(at, false, now)
    }

    /// Remember that a wait has looked at the room up to `seq` (forward only), so the next wait does not wake for
    /// the same messages. The read cursor is untouched: the messages stay unread. The default remembers nothing.
    fn wait_seen(&self, _at: Seat, _seq: i64, _now: i64) -> Result<()> {
        Ok(())
    }

    /// Messages from others past both the read cursor and the member's wake position (what a watcher has already
    /// pointed at). Changes no position; refreshes presence. The default is `read` without advancing.
    fn wake_poll(&self, at: Seat, now: i64) -> Result<Vec<Message>> {
        self.read(at, false, now)
    }

    /// Remember that a watcher pointed at the room up to `seq` (forward only). The default remembers nothing.
    fn wake_seen(&self, _at: Seat, _seq: i64, _now: i64) -> Result<()> {
        Ok(())
    }

    /// Messages from others that a hook has not shown yet, for the member's hook only. Remembers what it
    /// returned so the next hook call does not repeat it, but leaves the member's read cursor alone, so a
    /// message a hook showed can still be read once with `read` (#198). The default advances the read cursor.
    fn hook_read(&self, at: Seat, now: i64) -> Result<Vec<Message>> {
        self.read(at, true, now)
    }

    /// Messages with `seq` greater than `after_seq`, oldest first, at most `limit`, from every sender
    /// (including the caller, so `seq` has no gaps). No cursor and no presence side effects.
    fn tail(&self, room: &str, after_seq: i64, limit: usize) -> Result<Vec<Message>>;

    /// Every message of the room whose body starts with one of `prefixes` (exactly, case included), oldest first,
    /// at any depth in the log. The boards fold words (tasks, pull requests) that can be older than any window a
    /// `tail` reads, so they ask for the words alone. No cursor and no presence side effects. The default pages
    /// through `tail`; a store with an index or a query overrides it.
    fn words(&self, room: &str, prefixes: &[&str]) -> Result<Vec<Message>> {
        let mut found = Vec::new();
        if prefixes.is_empty() {
            return Ok(found);
        }
        let mut after = 0;
        loop {
            let page = self.tail(room, after, 5000)?;
            let Some(last) = page.last().map(|m| m.seq) else {
                return Ok(found);
            };
            after = last;
            found.extend(
                page.into_iter()
                    .filter(|m| prefixes.iter().any(|p| m.body.starts_with(p))),
            );
        }
    }

    /// The highest `seq` in a room, 0 when it is empty.
    fn head(&self, room: &str) -> Result<i64>;

    /// Move a member's read cursor forward to `seq`, never backward and never past the room's head.
    fn advance_cursor(&self, at: Seat, seq: i64, now: i64) -> Result<()>;
}
