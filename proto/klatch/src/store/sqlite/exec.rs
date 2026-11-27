//! The statement helpers every store method is written with, and the presence touch.

use super::{QueuedAudit, SqliteStore, be};
use crate::domain::Result;
use rusqlite::params;

const AUDIT_SQL: &str = "INSERT INTO admin_audit(ts,action,target,detail) VALUES(?1,?2,?3,?4)";

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
        self.owns_txn.set(true);
        // a panic in `f` unwinds through this guard, which rolls back (#1676)
        let guard = CloseOnUnwind(self);
        let out = f();
        std::mem::forget(guard);
        self.close_failed();
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

    /// Run `f` as one write transaction. Inside one already open (a caller's `atomically`), `f` joins it and the
    /// outer commit or rollback decides, so the single statements here nest without a second `BEGIN`.
    pub(super) fn immediate<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        if !self.conn.is_autocommit() {
            if self.owns_txn.get() {
                return f();
            }
            // an open transaction this store did not open is a dead one (a ROLLBACK failed, #1678): close it, and
            // refuse to run inside it if it will not close
            self.close_failed();
            if !self.conn.is_autocommit() {
                return Err(crate::domain::Error::Backend(
                    "a transaction left open by a failed rollback could not be closed".into(),
                ));
            }
        }
        self.conn.execute_batch("BEGIN IMMEDIATE").map_err(be)?;
        self.owns_txn.set(true);
        // a panic in `f` unwinds through this guard, which closes the transaction (#1591): a later write would
        // otherwise join the open one and never commit
        let guard = CloseOnUnwind(self);
        let out = match f() {
            Ok(v) => self.conn.execute_batch("COMMIT").map_err(be).map(|()| v),
            Err(e) => Err(e),
        };
        std::mem::forget(guard);
        self.close_failed();
        self.flush_deferred_audit();
        out
    }

    /// Roll back whatever transaction is still open: after a failed `f`, or a COMMIT that failed (BUSY, a deferred
    /// constraint) and left it open (#1592). A no-op once the transaction has closed.
    fn close_failed(&self) {
        if !self.conn.is_autocommit() && !self.rollback() {
            self.rollback_errors.set(self.rollback_errors.get() + 1);
        }
        self.owns_txn.set(false);
    }

    /// ROLLBACK; whether it succeeded.
    fn rollback(&self) -> bool {
        #[cfg(test)]
        if self.fail_rollbacks.get() > 0 {
            self.fail_rollbacks.set(self.fail_rollbacks.get() - 1);
            return false;
        }
        self.conn.execute_batch("ROLLBACK").is_ok()
    }

    /// Keep an audit row of a refused write: now when no transaction is open, else once the outermost one has
    /// closed, so a rollback cannot take it away (#1275).
    pub(super) fn audit_refusal(&self, action: &'static str, target: &str, now: i64) {
        if self.conn.is_autocommit() {
            if self
                .exec(AUDIT_SQL, params![now, action, target, ""])
                .is_err()
            {
                // queued for the next flush like a row made inside a transaction (#1677)
                self.count_audit_flush_error();
                self.requeue(vec![(action, target.to_string(), now, true)]);
            }
        } else {
            self.deferred_audit
                .borrow_mut()
                .push((action, target.to_string(), now, false));
        }
    }

    /// Audit rows of refused writes that could not be written (#1592).
    pub fn audit_flush_errors(&self) -> u64 {
        self.audit_flush_errors.get()
    }

    /// Refusal rows lost because the retry queue was full (#1677).
    pub fn audit_rows_dropped(&self) -> u64 {
        self.audit_rows_dropped.get()
    }

    /// ROLLBACKs that failed (#1678).
    pub fn rollback_errors(&self) -> u64 {
        self.rollback_errors.get()
    }

    fn count_audit_flush_error(&self) {
        self.audit_flush_errors
            .set(self.audit_flush_errors.get() + 1);
    }

    /// Write the queued refusal rows. One that cannot be written is counted (`audit_flush_errors`) and stays queued
    /// for the next flush, up to `MAX_QUEUED`; past that the oldest are dropped and counted apart.
    fn flush_deferred_audit(&self) {
        // a rollback that did not happen leaves the transaction open: the rows stay queued for the one that closes it
        if !self.conn.is_autocommit() {
            return;
        }
        let rows = std::mem::take(&mut *self.deferred_audit.borrow_mut());
        let mut kept = Vec::new();
        for (action, target, ts, failed) in rows {
            if self
                .exec(AUDIT_SQL, params![ts, action, target, ""])
                .is_err()
            {
                // a row counts once, however many flushes it fails (#1677)
                if !failed {
                    self.count_audit_flush_error();
                }
                kept.push((action, target, ts, true));
            }
        }
        self.requeue(kept);
    }

    /// Put `kept` (older) back in front of what was queued meanwhile; past `MAX_QUEUED` the oldest are dropped and
    /// counted (`audit_rows_dropped`).
    fn requeue(&self, mut kept: Vec<QueuedAudit>) {
        if kept.is_empty() {
            return;
        }
        let mut q = self.deferred_audit.borrow_mut();
        kept.append(&mut q);
        let over = kept.len().saturating_sub(MAX_QUEUED);
        kept.drain(..over);
        self.audit_rows_dropped
            .set(self.audit_rows_dropped.get() + over as u64);
        *q = kept;
    }
}

/// Closes the open transaction when the closure inside `immediate` or `snapshot` panics; forgotten on the normal path.
struct CloseOnUnwind<'a>(&'a SqliteStore);

impl Drop for CloseOnUnwind<'_> {
    fn drop(&mut self) {
        self.0.close_failed();
        self.0.flush_deferred_audit();
    }
}

/// How many audit rows that failed to write are held for a retry.
const MAX_QUEUED: usize = 256;
