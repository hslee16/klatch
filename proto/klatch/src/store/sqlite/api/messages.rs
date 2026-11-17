//! The SQLite side of the `MessageStore` area.

use super::super::{MESSAGE_COLUMNS, SqliteStore, be};
use crate::domain::{Message, Posted, Result};
use crate::store::MessageStore;
use crate::store::{NewMessage, Seat};
use rusqlite::params;

impl MessageStore for SqliteStore {
    fn join(&self, room: &str, member: &str, now: i64) -> Result<()> {
        self.touch(room, member, now)
    }

    fn post_as(&self, message: &NewMessage, now: i64) -> Result<Posted> {
        self.do_post_as(message, now)
    }

    fn read(&self, at: Seat, advance: bool, now: i64) -> Result<Vec<Message>> {
        self.do_read(at, advance, now)
    }

    fn wait_poll(&self, at: Seat, now: i64) -> Result<Vec<Message>> {
        self.do_wait_poll(at.room, at.member, now)
    }

    fn wait_seen(&self, at: Seat, seq: i64, now: i64) -> Result<()> {
        self.update_member(
            (at.room, at.member, now),
            "UPDATE members SET wait_seq=MAX(wait_seq,?3) WHERE room=?1 AND name=?2",
            &seq,
        )
    }

    fn wake_poll(&self, at: Seat, now: i64) -> Result<Vec<Message>> {
        self.do_wake_poll(at.room, at.member, now)
    }

    fn wake_seen(&self, at: Seat, seq: i64, now: i64) -> Result<()> {
        self.update_member(
            (at.room, at.member, now),
            "UPDATE members SET wake_seq=MAX(wake_seq,?3) WHERE room=?1 AND name=?2",
            &seq,
        )
    }

    fn hook_read(&self, at: Seat, now: i64) -> Result<Vec<Message>> {
        self.do_hook_read(at.room, at.member, now)
    }

    fn posts_since(&self, member: &str, since: i64) -> Result<Vec<i64>> {
        self.rows(
            "SELECT ts FROM messages WHERE sender=?1 AND ts>=?2 ORDER BY ts",
            params![member, since],
            |r| r.get(0),
        )
    }

    fn tail(&self, room: &str, after_seq: i64, limit: usize) -> Result<Vec<Message>> {
        self.rows(&format!(
                "SELECT {MESSAGE_COLUMNS} FROM messages WHERE room=?1 AND seq>?2 ORDER BY seq LIMIT ?3"
            ), params![room, after_seq, limit as i64], |r| self.open_message(r))
    }

    fn head(&self, room: &str) -> Result<i64> {
        self.conn
            .query_row(
                "SELECT COALESCE(MAX(seq),0) FROM messages WHERE room=?1",
                params![room],
                |r| r.get(0),
            )
            .map_err(be)
    }

    fn advance_cursor(&self, at: Seat, seq: i64, now: i64) -> Result<()> {
        let Seat { room, member } = at;
        // One statement, so it is atomic: forward only, and clamped to the room's head so an
        // acknowledgement can never skip a message that has not arrived yet.
        self.exec(
                "INSERT INTO members(room,name,joined,last_seen,last_seq)
                 VALUES(?1,?2,?4,?4,MAX(0,MIN(?3,(SELECT COALESCE(MAX(seq),0) FROM messages WHERE room=?1))))
                 ON CONFLICT(room,name) DO UPDATE SET
                   last_seq=MAX(last_seq,MIN(?3,(SELECT COALESCE(MAX(seq),0) FROM messages WHERE room=?1))),
                   last_seen=?4",
                params![room, member, seq, now],
            )?;
        Ok(())
    }
}
