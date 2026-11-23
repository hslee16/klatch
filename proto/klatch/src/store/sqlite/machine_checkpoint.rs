//! The machine checkpoint (#1408) in the store: the working trail between hooks and the latest checkpoint per room
//! and member. Paths and classes only; the content of a call never reaches this file.

use super::SqliteStore;
use crate::domain::{MachineCheckpoint, Result, Trail};
use crate::store::Seat;
use rusqlite::params;

/// A machine checkpoint to store: whose, in which room, its text and whether it is the last of the run.
#[derive(Clone, Copy)]
pub(super) struct CheckpointWrite<'a> {
    pub(super) room: &'a str,
    pub(super) member: &'a str,
    pub(super) body: &'a str,
    pub(super) last: bool,
}

/// A machine status to write: the text, its class word, and the texts the machine owns (only those may be replaced).
#[derive(Clone, Copy)]
pub(super) struct StatusWrite<'a> {
    pub(super) text: &'a str,
    pub(super) class: &'a str,
    pub(super) owned: &'a [&'a str],
}

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
        cp: &CheckpointWrite<'_>,
        now: i64,
    ) -> Result<bool> {
        let CheckpointWrite {
            room,
            member,
            body,
            last,
        } = *cp;
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

    /// Write the member's machine status in the room if it is due (see `Store::put_machine_status`) and count it.
    /// `status` is the text and its class word; `when` is now and the window, in seconds.
    pub(super) fn do_put_machine_status(
        &self,
        at: Seat,
        status: &StatusWrite<'_>,
        (now, window): (i64, i64),
    ) -> Result<bool> {
        let StatusWrite { text, class, owned } = *status;
        self.immediate(|| {
            let row: Option<(Option<String>, i64)> = self.optional(
                "SELECT status,status_ts FROM members WHERE room=?1 AND name=?2",
                params![at.room, at.member],
                |r| Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0))),
            )?;
            // a member that is not in the room has no status to set, and the machine does not join it
            let Some((current, set_at)) = row else {
                return Ok(false);
            };
            let due = match current.as_deref() {
                None => true,
                Some(t) if !owned.contains(&t) => false,
                Some(t) => t != text || now - set_at >= window,
            };
            if !due {
                return Ok(false);
            }
            self.exec(
                "UPDATE members SET status=?3, status_ts=?4 WHERE room=?1 AND name=?2",
                params![at.room, at.member, text, now],
            )?;
            self.exec(
                "INSERT INTO machine_status_events(ts,room,class) VALUES(?1,?2,?3)",
                params![now, at.room, class],
            )?;
            self.exec(
                "DELETE FROM machine_status_events WHERE ts<?1",
                params![now - EVENT_MAX_AGE_SECS],
            )?;
            self.exec(
                "DELETE FROM machine_status_events WHERE id <= (SELECT MAX(id) FROM machine_status_events) - ?1",
                params![EVENT_MAX_ROWS],
            )?;
            Ok(true)
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
