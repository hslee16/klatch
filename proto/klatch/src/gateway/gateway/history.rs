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
        let (messages, top) = if range.last.is_some() || range.since.is_none() {
            (self.newest_visible(room, (since, head), n)?, head)
        } else {
            self.visible_after(room, (since, head), n)?
        };
        // what follows the scanned window; hidden rows at its edge are no "more"
        let more = (head - top).max(0) as usize;
        let messages = self.mark_advisory(room, messages)?;
        Ok(History { messages, more })
    }

    /// The newest `n` visible rows after `since`: scan back a window at a time (#1008), so a canary or tombstone
    /// among the newest rows is stepped over instead of taking a place in the count.
    fn newest_visible(
        &self,
        room: &str,
        (since, head): (i64, i64),
        n: usize,
    ) -> Result<Vec<Message>> {
        let mut kept: Vec<Message> = Vec::new();
        let mut upper = head;
        while kept.len() < n && upper > since {
            let want = (n - kept.len()) as i64;
            let lower = since.max(upper - want);
            let rows = self.store.tail(room, lower, (upper - lower) as usize)?;
            let mut visible: Vec<Message> = rows.into_iter().filter(|m| !is_hidden(m)).collect();
            visible.append(&mut kept);
            kept = visible;
            upper = lower;
        }
        if kept.len() > n {
            kept.drain(..kept.len() - n);
        }
        Ok(kept)
    }

    /// Paging forward from `since`: scan on until the page holds `n` visible rows or the room ends. Returns the
    /// page and the last seq scanned.
    fn visible_after(
        &self,
        room: &str,
        (since, head): (i64, i64),
        n: usize,
    ) -> Result<(Vec<Message>, i64)> {
        let mut kept: Vec<Message> = Vec::new();
        let mut scanned = since;
        while kept.len() < n && scanned < head {
            let rows = self.store.tail(room, scanned, n - kept.len())?;
            let Some(last) = rows.last().map(|m| m.seq) else {
                break;
            };
            scanned = last;
            kept.extend(rows.into_iter().filter(|m| !is_hidden(m)));
        }
        Ok((kept, scanned))
    }
}

/// An archived message is a tombstone in the live store; `read` leaves it out, so does `history`, and so a canary.
fn is_hidden(m: &Message) -> bool {
    matches!(
        m.kind.as_deref(),
        Some("archived") | Some(super::tripwire::CANARY_KIND)
    )
}
