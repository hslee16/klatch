//! What a member is told about asks, once: asks of its own gone stale, escalated, or whose claim lapsed. Each report
//! is remembered in a table so the same ask is not reported twice.

use super::SqliteStore;
use klatch_core::{OpenAsk, Result};
use rusqlite::params;

/// An open ask of `member`'s that nobody has claimed for `after` seconds and that is not waiting on something
/// else (its asker owing an answer, or a working room in use).
pub(super) fn is_stale(a: &OpenAsk, member: &str, now: i64, after: i64) -> bool {
    let waiting = a.claimed || a.waiting_on_asker.is_some() || a.room_active(now);
    a.sender == member && !waiting && now - a.ts >= after
}

impl SqliteStore {
    /// The `member`'s own open asks that `wanted` picks and that were not reported to it before in `table`;
    /// remembers what it returns.
    pub(super) fn report_once(
        &self,
        table: &str,
        (room, member): (&str, &str),
        asks: Vec<OpenAsk>,
    ) -> Result<Vec<OpenAsk>> {
        let sql = format!("INSERT OR IGNORE INTO {table}(room,member,seq) VALUES(?1,?2,?3)");
        let mut fresh = Vec::new();
        for a in asks {
            if self.exec(&sql, params![room, member, a.seq])? == 1 {
                fresh.push(a);
            }
        }
        Ok(fresh)
    }

    /// The holder and expiry of an ask's lease, if it has one.
    pub(super) fn ask_lease(&self, room: &str, seq: i64) -> Result<Option<(String, i64)>> {
        self.optional(
            "SELECT owner,expires FROM ask_leases WHERE room=?1 AND seq=?2",
            params![room, seq],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    }

    /// Remember that `member` was told its ask's lease ran out at `expires`; whether this is the first time.
    pub(super) fn report_lapse_once(
        &self,
        (room, member): (&str, &str),
        seq: i64,
        expires: i64,
    ) -> Result<bool> {
        let n = self.exec(
            "INSERT OR IGNORE INTO asks_lapse_reported(room,member,seq,expires) VALUES(?1,?2,?3,?4)",
            params![room, member, seq, expires],
        )?;
        Ok(n == 1)
    }
}
