//! The machine checkpoint (#1408) in the store: the working trail between hooks and the latest checkpoint per room
//! and member. Paths and classes only; the content of a call never reaches this file.

use super::SqliteStore;
use crate::domain::{MachineCheckpoint, Result, Trail};
use rusqlite::params;

/// Counts rows older than this are deleted as events are written: 30 days, like the claim events.
const EVENT_MAX_AGE_SECS: i64 = 30 * 86_400;
/// The event table keeps at most this many rows, the newest.
const EVENT_MAX_ROWS: i64 = 20_000;

impl SqliteStore {
    pub(super) fn do_trail_of(&self, member: &str) -> Result<Trail> {
        let row: Option<(String, String)> = self.optional(
            "SELECT files,tools FROM member_trail WHERE member=?1",
            params![member],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let split = |s: &str, sep: char| -> Vec<String> {
            s.split(sep)
                .filter(|w| !w.is_empty())
                .map(String::from)
                .collect()
        };
        Ok(row.map_or_else(Trail::default, |(files, tools)| Trail {
            files: split(&files, '\n'),
            tools: split(&tools, ' '),
        }))
    }

    pub(super) fn do_save_trail(&self, member: &str, trail: &Trail, now: i64) -> Result<()> {
        self.write(
            "INSERT INTO member_trail(member,files,tools,ts) VALUES(?1,?2,?3,?4)
             ON CONFLICT(member) DO UPDATE SET files=?2, tools=?3, ts=?4",
            params![member, trail.files.join("\n"), trail.tools.join(" "), now],
        )
    }

    pub(super) fn do_clear_trail(&self, member: &str) -> Result<()> {
        self.write("DELETE FROM member_trail WHERE member=?1", params![member])
    }

    /// Replace the member's checkpoint in the room; whether there was one to replace.
    pub(super) fn do_put_machine_checkpoint(
        &self,
        (room, member): (&str, &str),
        body: &str,
        last: bool,
        now: i64,
    ) -> Result<bool> {
        self.immediate(|| {
            let replaced = self.exec(
                "DELETE FROM machine_checkpoints WHERE room=?1 AND member=?2",
                params![room, member],
            )? > 0;
            self.exec(
                "INSERT INTO machine_checkpoints(room,member,body,ts,last) VALUES(?1,?2,?3,?4,?5)",
                params![room, member, body, now, last],
            )?;
            self.exec(
                "INSERT INTO machine_checkpoint_events(ts,room,replaced,last,bytes) VALUES(?1,?2,?3,?4,?5)",
                params![now, room, replaced, last, body.len() as i64],
            )?;
            self.exec(
                "DELETE FROM machine_checkpoint_events WHERE ts<?1",
                params![now - EVENT_MAX_AGE_SECS],
            )?;
            self.exec(
                "DELETE FROM machine_checkpoint_events WHERE id <= (SELECT MAX(id) FROM machine_checkpoint_events) - ?1",
                params![EVENT_MAX_ROWS],
            )?;
            Ok(replaced)
        })
    }

    pub(super) fn do_machine_checkpoints(&self, room: &str) -> Result<Vec<MachineCheckpoint>> {
        self.rows(
            "SELECT room,member,body,ts,last FROM machine_checkpoints WHERE room=?1 ORDER BY member",
            params![room],
            |r| {
                Ok(MachineCheckpoint {
                    room: r.get(0)?,
                    member: r.get(1)?,
                    body: r.get(2)?,
                    ts: r.get(3)?,
                    last: r.get::<_, i64>(4)? != 0,
                })
            },
        )
    }
}
