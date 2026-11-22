//! Which members pruning must spare, and the notes released claims leave.

use super::SqliteStore;
use crate::domain::{Message, Result};
use crate::store::ClaimStore;
use rusqlite::params;

/// The messages of a result ordered by room, collected per room (a room's run is consecutive).
fn per_room(rows: Vec<(String, Message)>) -> Vec<Vec<Message>> {
    let mut rooms: Vec<Vec<Message>> = Vec::new();
    let mut room = String::new();
    for (r, m) in rows {
        if r != room || rooms.is_empty() {
            room = r;
            rooms.push(Vec::new());
        }
        if let Some(last) = rooms.last_mut() {
            last.push(m);
        }
    }
    rooms
}

/// Owners of the open tasks of each room that had a word within `TASK_HOLD_SECS` of `now`.
fn held_owners(rooms: Vec<Vec<Message>>, now: i64) -> Vec<String> {
    rooms
        .iter()
        .flat_map(|msgs| crate::domain::open_tasks(msgs))
        .filter(|t| now - t.last <= crate::domain::TASK_HOLD_SECS)
        .map(|t| t.owner)
        .collect()
}

impl SqliteStore {
    /// Members pruning must spare because they may come back to work they hold: those with a live claim in any
    /// room (DESIGN section 32 item 44), a park that holds (item 46) and an open task with a word in the last
    /// `TASK_HOLD_SECS` (item 48).
    pub(super) fn members_that_may_return(&self, now: i64) -> Result<Vec<String>> {
        let mut keep: Vec<String> = self.rows(
            "SELECT DISTINCT owner FROM claims WHERE expires>?1
             UNION SELECT member FROM member_parked WHERE back_by>?1",
            params![now],
            |r| r.get(0),
        )?;
        keep.extend(self.task_owners(now)?);
        Ok(keep)
    }

    /// Owners of open tasks that had a word within `TASK_HOLD_SECS`. Only the task words are loaded, per room, and
    /// folded with the same rules the board uses.
    pub(super) fn task_owners(&self, now: i64) -> Result<Vec<String>> {
        let sql = "SELECT room,seq,sender,body,ts FROM messages WHERE body LIKE 'working:%' OR body LIKE 'checkpoint%'
             OR body LIKE 'handoff:%' OR body LIKE 'finished%' ORDER BY room,seq";
        let rows: Vec<(String, Message)> = self.rows(sql, [], |r| {
            Ok((
                r.get(0)?,
                Message {
                    id: String::new(),
                    seq: r.get(1)?,
                    sender: r.get(2)?,
                    attribution: crate::domain::Attribution::Unattributed,
                    label: String::new(),
                    body: r.get(3)?,
                    ts: r.get(4)?,
                    kind: None,
                    payload: None,
                    advisory: false,
                    scope: None,
                },
            ))
        })?;
        Ok(held_owners(per_room(rows), now))
    }

    /// The note `owner` left when it released `path`, if it was the latest release of that path (#272).
    pub(super) fn note_of(&self, room: &str, owner: &str, path: &str) -> Result<Option<String>> {
        Ok(self
            .last_release(room, path)?
            .filter(|n| n.owner == owner)
            .map(|n| n.note))
    }
}
