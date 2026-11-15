//! `history` (#518): the last messages of a room, from every sender, for a member (or the librarian) who wants to
//! look back. It reads through the store's `tail`, so a sealed label's body is opened or shown as a placeholder
//! exactly as `tail` and `read` do, and it asks the guard as a non-advancing read, so the clearance, stop, closed
//! room and role checks of `read --peek` apply and a librarian may use it. It moves no cursor and refreshes no
//! presence.

use super::*;
use crate::domain::budget::{HISTORY_DEFAULT_MESSAGES, HISTORY_MAX_MESSAGES};

/// Which messages `history` returns. Neither field: the newest 20. `last` alone: the newest `last`. `since` alone:
/// the next 20 after `since` (to page forward). Both: the newest `last` after `since`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HistoryRange {
    pub since: Option<i64>,
    pub last: Option<usize>,
}

/// What `history` found, oldest first, and how many messages follow the last one returned (0 when it reaches the
/// room's newest). Paging forward is `since` the last returned `seq`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    pub messages: Vec<Message>,
    pub more: usize,
}

impl<S: Store> Gateway<S> {
    /// Messages of `room` in `range`, at most `HISTORY_MAX_MESSAGES`. Moves no cursor.
    pub fn history(&self, who: &Principal, room: &str, range: HistoryRange) -> Result<History> {
        if range.since.is_some_and(|s| s < 0) || range.last == Some(0) {
            return Err(Error::Invalid("bad history range".into()));
        }
        self.admit(who, room, Operation::read(room, false))?;
        let n = range
            .last
            .unwrap_or(HISTORY_DEFAULT_MESSAGES)
            .min(HISTORY_MAX_MESSAGES);
        let since = range.since.unwrap_or(0);
        let head = self.store.head(room)?;
        // the newest `n` unless the caller only gave `since`, which pages forward from it
        let start = if range.last.is_some() || range.since.is_none() {
            since.max(head - n as i64)
        } else {
            since
        };
        let rows = self.store.tail(room, start, n)?;
        let more = rows.last().map_or(0, |m| (head - m.seq).max(0)) as usize;
        // an archived message is a tombstone in the live store; `read` leaves it out, so does this
        let messages = rows
            .into_iter()
            .filter(|m| m.kind.as_deref() != Some("archived"))
            .collect();
        Ok(History { messages, more })
    }
}
