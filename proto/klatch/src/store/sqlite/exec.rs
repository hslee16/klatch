//! The statement helpers every store method is written with, and the presence touch.

use super::{SqliteStore, be};
use crate::domain::Result;
use rusqlite::params;

impl SqliteStore {
    /// Mark the member present: last-seen is a whole second, so a touch in the second already recorded changes nothing
    /// and writes nothing (#128); at most one write per member and room each second, however often it is called.
    pub(super) fn touch(&self, room: &str, me: &str, now: i64) -> Result<()> {
        if self.last_seen(room, me)? != Some(now) {
            self.exec(
                "INSERT INTO members(room,name,joined,last_seen) VALUES(?1,?2,?3,?3)
                     ON CONFLICT(room,name) DO UPDATE SET last_seen=?3",
                params![room, me, now],
            )?;
        }
        self.renew_claims(room, me, now)
    }

    /// When the member was last seen in the room, if it has a row.
    pub(super) fn last_seen(&self, room: &str, me: &str) -> Result<Option<i64>> {
        self.optional(
            "SELECT last_seen FROM members WHERE room=?1 AND name=?2",
            params![room, me],
            |r| r.get(0),
        )
    }

    /// Run `f` in one read transaction: a consistent snapshot that never takes the writer lock. `f` must only read.
    pub(super) fn snapshot<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        self.conn.execute_batch("BEGIN").map_err(be)?;
        let out = f();
        let _ = self.conn.execute_batch("ROLLBACK");
        out
    }

    /// A member that is active keeps each live claim in the room alive for the ttl that claim was made with, counted
    /// from now, so a claim does not lapse mid-task (#494). Never a floor: a 60 s claim stays 60 s. It never shortens
    /// a lease (a liveness lease or a park may have stretched it), never revives a lapsed claim, leaves `ask:`
    /// leases to their own cap, and skips a claim whose ttl is unknown (stored 0, made before the column existed).
    pub(super) fn renew_claims(&self, room: &str, me: &str, now: i64) -> Result<()> {
        for (path, expires) in self.due_renewals(room, me, now)? {
            self.exec(
                "UPDATE claims SET expires=?4 WHERE room=?1 AND owner=?2 AND path=?3",
                params![room, me, path, expires],
            )?;
            // What others were shown moves with the claim: a heartbeat is not news, and a digest reports a claim
            // whose expiry differs from the one a member saw.
            self.exec(
                "UPDATE claims_seen SET expires=?4 WHERE room=?1 AND owner=?2 AND path=?3 AND expires<?4",
                params![room, me, path, expires],
            )?;
        }
        Ok(())
    }

    /// The claims `renew_claims` would move, with their new expiry. A read.
    pub(super) fn due_renewals(
        &self,
        room: &str,
        me: &str,
        now: i64,
    ) -> Result<Vec<(String, i64)>> {
        self.rows(
            "SELECT path, ?3+ttl FROM claims WHERE room=?1 AND owner=?2 AND ttl>0 AND expires>?3
                 AND expires<?3+ttl AND path NOT LIKE 'ask:%'",
            params![room, me, now],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    }

    /// Run one write statement in its own transaction.
    pub(super) fn write(&self, sql: &str, params: impl rusqlite::Params) -> Result<()> {
        self.immediate(|| self.exec(sql, params).map(|_| ()))
    }

    /// Mark the member present in the room, then update its row: `sql` takes the room, the member and `value`.
    pub(super) fn update_member(
        &self,
        (room, member, now): (&str, &str, i64),
        sql: &str,
        value: &dyn rusqlite::ToSql,
    ) -> Result<()> {
        self.immediate(|| {
            self.touch(room, member, now)?;
            self.exec(sql, params![room, member, value]).map(|_| ())
        })
    }

    /// Run one statement; how many rows it changed.
    pub(super) fn exec(&self, sql: &str, params: impl rusqlite::Params) -> Result<usize> {
        self.conn.execute(sql, params).map_err(be)
    }

    /// The first row of a query, if there is one.
    pub(super) fn optional<T>(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
        map: impl FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Option<T>> {
        match self.conn.query_row(sql, params, map) {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(be(e)),
        }
    }

    /// Every row of a query.
    pub(super) fn rows<T>(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
        map: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Vec<T>> {
        let mut st = self.conn.prepare(sql).map_err(be)?;
        st.query_map(params, map)
            .map_err(be)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(be)
    }

    pub(super) fn immediate<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        self.conn.execute_batch("BEGIN IMMEDIATE").map_err(be)?;
        match f() {
            Ok(v) => {
                self.conn.execute_batch("COMMIT").map_err(be)?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }
}
